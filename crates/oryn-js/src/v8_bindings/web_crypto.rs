//! Native WebCrypto subset required by the pinned Core dependency inventory.
//! Author-visible descriptors never authorize key use; immutable native records do.
use super::*;
use crate::crypto_key::{Key, MAX_DATA, Usage};
use base64::Engine;
use zeroize::Zeroizing;
fn error(name: &str) -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "WebCrypto".into(),
        member: name.into(),
    }
}
fn text(scope: &mut v8::PinScope, value: v8::Local<v8::Value>) -> Result<String, V8BindingError> {
    Ok(value
        .to_string(scope)
        .ok_or(V8BindingError::JavaScriptException)?
        .to_rust_string_lossy(scope))
}
fn property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let key = v8::String::new(scope, name).ok_or(V8BindingError::Allocation("crypto property"))?;
    object
        .get(scope, key.into())
        .ok_or(V8BindingError::JavaScriptException)
}
fn put<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
    value: v8::Local<'s, v8::Value>,
) -> Result<(), V8BindingError> {
    let key = v8::String::new(scope, name).ok_or(V8BindingError::Allocation("crypto property"))?;
    if object.create_data_property(scope, key.into(), value) != Some(true) {
        return Err(V8BindingError::Allocation("crypto property"));
    }
    Ok(())
}
fn algorithm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<String, V8BindingError> {
    let value = if value.is_object() {
        let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| error("TypeError"))?;
        let name = property(scope, object, "name")?;
        if name.is_undefined() {
            return Err(error("TypeError"));
        }
        name
    } else {
        value
    };
    let name = text(scope, value)?;
    Ok(match name.to_ascii_uppercase().as_str() {
        "AES-CBC" => "AES-CBC".into(),
        "ED25519" => "Ed25519".into(),
        "SHA-1" => "SHA-1".into(),
        "SHA-256" => "SHA-256".into(),
        "SHA-384" => "SHA-384".into(),
        "SHA-512" => "SHA-512".into(),
        _ => return Err(error("NotSupportedError")),
    })
}
fn aes_algorithm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<(), V8BindingError> {
    if algorithm(scope, value)? != "AES-CBC" {
        return Err(error("NotSupportedError"));
    }
    Ok(())
}
fn dictionary<'s>(
    value: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    v8::Local::<v8::Object>::try_from(value).map_err(|_| error("TypeError"))
}
fn bytes(scope: &mut v8::PinScope, value: v8::Local<v8::Value>) -> Result<Vec<u8>, V8BindingError> {
    if let Ok(view) = v8::Local::<v8::ArrayBufferView>::try_from(value) {
        let buffer = view.buffer(scope).ok_or_else(|| error("TypeError"))?;
        if buffer.was_detached()
            || buffer.get_backing_store().is_shared()
            || buffer.get_backing_store().is_resizable_by_user_javascript()
        {
            return Err(error("TypeError"));
        }
        if view.byte_length() > MAX_DATA {
            return Err(error("OperationError"));
        }
        let mut output = vec![0; view.byte_length()];
        view.copy_contents(&mut output);
        return Ok(output);
    }
    let buffer = v8::Local::<v8::ArrayBuffer>::try_from(value).map_err(|_| error("TypeError"))?;
    if buffer.was_detached() || buffer.get_backing_store().is_resizable_by_user_javascript() {
        return Err(error("TypeError"));
    }
    if buffer.byte_length() > MAX_DATA {
        return Err(error("OperationError"));
    }
    Ok(bytes_from_array_buffer(buffer))
}
fn iv<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<Vec<u8>, V8BindingError> {
    aes_algorithm(scope, value)?;
    let object = dictionary(value)?;
    let value = property(scope, object, "iv")?;
    let bytes = bytes(scope, value)?;
    if bytes.len() != 16 {
        return Err(error("OperationError"));
    }
    Ok(bytes)
}
fn usages(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<Vec<Usage>, V8BindingError> {
    let object = dictionary(value)?;
    let iterator = v8::Symbol::get_iterator(scope);
    let iterator = object
        .get(scope, iterator.into())
        .ok_or(V8BindingError::JavaScriptException)?;
    let iterator = v8::Local::<v8::Function>::try_from(iterator).map_err(|_| error("TypeError"))?;
    let mut output = Vec::new();
    let mut count = 0;
    iterable_conversion::consume(scope, object, iterator, |scope, value| {
        count += 1;
        if count > 1024 {
            return Err(error("QuotaExceededError"));
        }
        let name = text(scope, value)?;
        let usage = match name.as_str() {
            "encrypt" => Usage::Encrypt,
            "decrypt" => Usage::Decrypt,
            "wrapKey" => Usage::WrapKey,
            "unwrapKey" => Usage::UnwrapKey,
            "sign" => Usage::Sign,
            "verify" => Usage::Verify,
            "deriveKey" | "deriveBits" => return Err(error("SyntaxError")),
            _ => return Err(error("TypeError")),
        };
        if !output.contains(&usage) {
            output.push(usage);
        }
        Ok(())
    })?;
    Ok(output)
}
fn native_key(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<std::sync::Arc<Key>, V8BindingError> {
    let object = dictionary(value)?;
    let owner = binding_state_for_receiver(scope, object).ok_or_else(|| error("TypeError"))?;
    let id = stateful_id(scope, object).map_err(|_| error("TypeError"))?;
    if !owner
        .platform_wrappers
        .borrow()
        .get(&id)
        .and_then(|value| value.to_local(scope))
        .is_some_and(|value| value.strict_equals(object.into()))
    {
        return Err(error("TypeError"));
    }
    let record = platform_object(&owner, id)?;
    match &*record {
        PlatformObjectRecord::CryptoKey(key) => Ok(key.clone()),
        _ => Err(error("TypeError")),
    }
}
pub(super) fn materialize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    key: Key,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    if !state.secure_context.get() {
        return Err(error("DataCloneError"));
    }
    key.validate().map_err(error)?;
    platform_listener_graph::reclaim(state);
    if state
        .platform_objects
        .borrow()
        .values()
        .filter(|record| matches!(record, PlatformObjectRecord::CryptoKey(_)))
        .count()
        >= 4096
    {
        return Err(error("QuotaExceededError"));
    }
    let id = register_platform_object(
        state,
        PlatformObjectRecord::CryptoKey(std::sync::Arc::new(key)),
    );
    match new_platform_object(scope, state, "CryptoKey", id) {
        Ok(object) => Ok(object),
        Err(error) => {
            state.platform_objects.borrow_mut().remove(&id);
            Err(error)
        }
    }
}
pub(super) fn release_collected_key(state: &V8BindingState, id: u64) {
    let key = matches!(
        state.platform_objects.borrow().get(&id),
        Some(PlatformObjectRecord::CryptoKey(_))
    );
    if key {
        state.platform_objects.borrow_mut().remove(&id);
        state.platform_wrappers.borrow_mut().remove(&id);
    }
}
fn cached<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &str,
) -> Result<(v8::Local<'s, v8::Private>, Option<v8::Local<'s, v8::Value>>), V8BindingError> {
    let name = v8::String::new(scope, name).ok_or(V8BindingError::Allocation("crypto cache"))?;
    let key = v8::Private::for_api(scope, Some(name));
    let value = object
        .get_private(scope, key)
        .filter(|value| !value.is_undefined());
    Ok((key, value))
}
pub(super) fn get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<'s, v8::Object>,
    interface: &str,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    if interface == "Crypto" {
        if member != "subtle" {
            return Err(error("TypeError"));
        }
        let id = stateful_id(scope, object)?;
        let subtle_id = match &*platform_object(state, id)? {
            PlatformObjectRecord::Crypto { subtle_id } => *subtle_id,
            _ => return Err(error("TypeError")),
        };
        if !state.secure_context.get() {
            return Ok(v8::undefined(scope).into());
        }
        let (key, value) = cached(scope, object, "oryn.crypto.subtle")?;
        if let Some(value) = value {
            return Ok(value);
        }
        let subtle = new_platform_object(scope, state, "SubtleCrypto", subtle_id)?;
        object
            .set_private(scope, key, subtle.into())
            .ok_or(V8BindingError::Allocation("crypto subtle cache"))?;
        return Ok(subtle.into());
    }
    let key = native_key(scope, object.into())?;
    let context = object
        .get_creation_context(scope)
        .ok_or(V8BindingError::Allocation("key descriptor realm"))?;
    let scope = &mut v8::ContextScope::new(scope, context);
    match member {
        "type" => v8_string(scope, key.kind()),
        "extractable" => Ok(v8::Boolean::new(scope, key.extractable()).into()),
        "algorithm" | "usages" => {
            let (cache, value) = cached(
                scope,
                object,
                if member == "algorithm" {
                    "oryn.cryptokey.algorithm"
                } else {
                    "oryn.cryptokey.usages"
                },
            )?;
            if let Some(value) = value {
                return Ok(value);
            }
            let value = if member == "algorithm" {
                let algorithm = v8::Object::new(scope);
                let name = v8_string(scope, key.algorithm())?;
                put(scope, algorithm, "name", name)?;
                if key.algorithm() == "AES-CBC" {
                    let length = v8::Integer::new(scope, key.bits() as i32);
                    put(scope, algorithm, "length", length.into())?;
                }
                algorithm.into()
            } else {
                let array = v8::Array::new(scope, key.usages().len() as i32);
                for (index, usage) in key.usages().iter().enumerate() {
                    let value = v8_string(scope, usage.name())?;
                    array
                        .set_index(scope, index as u32, value)
                        .ok_or(V8BindingError::Allocation("key usages"))?;
                }
                array
                    .set_integrity_level(scope, v8::IntegrityLevel::Frozen)
                    .ok_or(V8BindingError::Allocation("key usages freeze"))?;
                array.into()
            };
            object
                .set_private(scope, cache, value)
                .ok_or(V8BindingError::Allocation("key descriptor cache"))?;
            Ok(value)
        }
        _ => Err(error("TypeError")),
    }
}
fn jwk_value(key: &Key) -> Result<serde_json::Value, V8BindingError> {
    let bytes = key.export().map_err(error)?;
    let mut object = serde_json::Map::new();
    if key.algorithm() == "AES-CBC" {
        object.insert("kty".into(), "oct".into());
        object.insert(
            "k".into(),
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(&bytes)
                .into(),
        );
        object.insert("alg".into(), format!("A{}CBC", key.bits()).into());
    } else {
        object.insert("kty".into(), "OKP".into());
        object.insert("crv".into(), "Ed25519".into());
        object.insert("alg".into(), "Ed25519".into());
        object.insert(
            "x".into(),
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(key.public_bytes().map_err(error)?)
                .into(),
        );
        if key.kind() == "private" {
            object.insert(
                "d".into(),
                base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .encode(&bytes)
                    .into(),
            );
        }
    }
    object.insert("ext".into(), key.extractable().into());
    object.insert(
        "key_ops".into(),
        key.usages()
            .iter()
            .map(|usage| serde_json::Value::from(usage.name()))
            .collect(),
    );
    Ok(object.into())
}
fn exported_bytes(format: &str, key: &Key) -> Result<Zeroizing<Vec<u8>>, V8BindingError> {
    match format {
        "jwk" => serde_json::to_vec(&jwk_value(key)?)
            .map(Zeroizing::new)
            .map_err(|_| error("OperationError")),
        "pkcs8" | "spki" => key.export_der(format).map_err(error),
        "raw" | "raw-public" | "raw-secret" | "raw-seed" => {
            let valid = match format {
                "raw" => key.kind() != "private",
                "raw-public" => key.kind() == "public",
                "raw-secret" => key.kind() == "secret",
                "raw-seed" => key.kind() == "private",
                _ => false,
            };
            if !valid {
                return Err(error("InvalidAccessError"));
            }
            key.export().map_err(error)
        }
        _ => Err(error("NotSupportedError")),
    }
}
fn export<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    format: &str,
    key: &Key,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let bytes = exported_bytes(format, key)?;
    if format == "jwk" {
        let source = v8::String::new(
            scope,
            std::str::from_utf8(&bytes).map_err(|_| error("OperationError"))?,
        )
        .ok_or(V8BindingError::Allocation("JWK output"))?;
        v8::json::parse(scope, source).ok_or(V8BindingError::Allocation("JWK output"))
    } else {
        Ok(array_buffer_from_bytes(scope, &bytes).into())
    }
}
fn jwk_operations(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<Vec<String>, V8BindingError> {
    let object = dictionary(value)?;
    let key = v8::Symbol::get_iterator(scope);
    let method = object
        .get(scope, key.into())
        .ok_or(V8BindingError::JavaScriptException)?;
    let method = v8::Local::<v8::Function>::try_from(method).map_err(|_| error("TypeError"))?;
    let mut values = Vec::new();
    iterable_conversion::consume(scope, object, method, |scope, value| {
        if values.len() >= 8 {
            return Err(error("DataError"));
        }
        values.push(text(scope, value)?);
        Ok(())
    })?;
    Ok(values)
}
// Convert supported JWK dictionary members once, before algorithm normalization can
// invoke author code. The resulting own-data snapshot cannot re-read author getters.
fn snapshot_key_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    format: &str,
    data: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    if format != "jwk" {
        let copied = Zeroizing::new(bytes(scope, data)?);
        return Ok(array_buffer_from_bytes(scope, &copied).into());
    }
    let source = dictionary(data)?;
    let result = v8::Object::new(scope);
    for name in ["alg", "crv", "d", "ext", "k", "key_ops", "kty", "use", "x"] {
        let value = property(scope, source, name)?;
        if value.is_undefined() {
            put(scope, result, name, value)?;
            continue;
        }
        let value = if name == "ext" {
            {
                let flag = value.boolean_value(scope);
                v8::Boolean::new(scope, flag).into()
            }
        } else if name == "key_ops" {
            let values = jwk_operations(scope, value)?;
            let array = v8::Array::new(scope, values.len() as i32);
            for (index, value) in values.iter().enumerate() {
                let value = v8_string(scope, value)?;
                if array.set_index(scope, index as u32, value) != Some(true) {
                    return Err(V8BindingError::Allocation("JWK usage snapshot"));
                }
            }
            array.into()
        } else {
            let value = text(scope, value)?;
            v8_string(scope, &value)?
        };
        put(scope, result, name, value)?;
    }
    Ok(result.into())
}
fn import_jwk<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    data: v8::Local<'s, v8::Value>,
    algorithm: &str,
    extractable: bool,
    usages: Vec<Usage>,
) -> Result<Key, V8BindingError> {
    let object = dictionary(data)?;
    let kty = property(scope, object, "kty")?;
    if text(scope, kty)? != if algorithm == "AES-CBC" { "oct" } else { "OKP" } {
        return Err(error("DataError"));
    }
    let decode = |scope: &mut v8::PinScope,
                  value: v8::Local<v8::Value>|
     -> Result<Zeroizing<Vec<u8>>, V8BindingError> {
        let encoded = text(scope, value)?;
        if encoded.len() > 64 {
            return Err(error("DataError"));
        }
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded)
            .map(Zeroizing::new)
            .map_err(|_| error("DataError"))
    };
    let (material, private, public) = if algorithm == "AES-CBC" {
        let value = property(scope, object, "k")?;
        (decode(scope, value)?, None, None)
    } else {
        let crv = property(scope, object, "crv")?;
        if text(scope, crv)? != "Ed25519" {
            return Err(error("DataError"));
        }
        let value = property(scope, object, "x")?;
        let public = decode(scope, value)?;
        if public.len() != 32 {
            return Err(error("DataError"));
        }
        let secret = property(scope, object, "d")?;
        if secret.is_undefined() {
            (public, Some(false), None)
        } else {
            (decode(scope, secret)?, Some(true), Some(public))
        }
    };
    let alg = property(scope, object, "alg")?;
    if !alg.is_undefined() {
        let alg = text(scope, alg)?;
        if if algorithm == "AES-CBC" {
            alg != format!("A{}CBC", material.len() * 8)
        } else {
            !matches!(alg.as_str(), "Ed25519" | "EdDSA")
        } {
            return Err(error("DataError"));
        }
    }
    let ext = property(scope, object, "ext")?;
    if !ext.is_undefined() && !ext.boolean_value(scope) && extractable {
        return Err(error("DataError"));
    }
    let use_ = property(scope, object, "use")?;
    if !use_.is_undefined()
        && text(scope, use_)? != if algorithm == "AES-CBC" { "enc" } else { "sig" }
    {
        return Err(error("DataError"));
    }
    let ops = property(scope, object, "key_ops")?;
    if !ops.is_undefined() {
        let values = jwk_operations(scope, ops)?;
        let mut seen = std::collections::BTreeSet::new();
        if values.len() > 8
            || values.iter().any(|value| !seen.insert(value.as_str()))
            || usages
                .iter()
                .any(|usage| !values.iter().any(|value| value == usage.name()))
        {
            return Err(error("DataError"));
        }
    }
    if usages.is_empty() && private != Some(false) {
        return Err(error("SyntaxError"));
    }
    let result = (if let Some(private) = private {
        if usages
            .iter()
            .any(|usage| *usage != if private { Usage::Sign } else { Usage::Verify })
        {
            return Err(error("SyntaxError"));
        }
        Key::ed25519(material.to_vec(), private, extractable, usages)
    } else {
        if usages
            .iter()
            .any(|usage| matches!(usage, Usage::Sign | Usage::Verify))
        {
            return Err(error("SyntaxError"));
        }
        Key::new(material.to_vec(), extractable, usages)
    })
    .map_err(error)?;
    if public.is_some_and(|public| {
        result
            .public_bytes()
            .ok()
            .is_none_or(|value| value.as_slice() != public.as_slice())
    }) {
        return Err(error("DataError"));
    }
    Ok(result)
}
fn import<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    format: &str,
    data: v8::Local<'s, v8::Value>,
    algorithm: &str,
    extractable: bool,
    usages: Vec<Usage>,
) -> Result<Key, V8BindingError> {
    if !matches!(algorithm, "AES-CBC" | "Ed25519") {
        return Err(error("NotSupportedError"));
    }
    if format == "jwk" {
        return import_jwk(scope, data, algorithm, extractable, usages);
    }
    let material = Zeroizing::new(bytes(scope, data)?);
    if algorithm == "AES-CBC" {
        if !matches!(format, "raw" | "raw-secret") {
            return Err(error("NotSupportedError"));
        }
        if usages.is_empty()
            || usages
                .iter()
                .any(|usage| matches!(usage, Usage::Sign | Usage::Verify))
        {
            return Err(error("SyntaxError"));
        }
        return Key::new(material.to_vec(), extractable, usages).map_err(error);
    }
    let private = matches!(format, "pkcs8" | "raw-seed");
    if !matches!(format, "raw" | "raw-public" | "raw-seed" | "pkcs8" | "spki") {
        return Err(error("NotSupportedError"));
    }
    if private && usages.is_empty()
        || usages
            .iter()
            .any(|usage| *usage != if private { Usage::Sign } else { Usage::Verify })
    {
        return Err(error("SyntaxError"));
    }
    if matches!(format, "pkcs8" | "spki") {
        Key::import_der(format, &material, extractable, usages).map_err(error)
    } else {
        Key::ed25519(material.to_vec(), private, extractable, usages).map_err(error)
    }
}
fn compute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    match member {
        "generateKey" => {
            let usages = usages(scope, args.get(2))?;
            let name = algorithm(scope, args.get(0))?;
            if name == "Ed25519" {
                let (public, private) =
                    Key::generate_ed25519(args.get(1).boolean_value(scope), usages)
                        .map_err(error)?;
                let public = materialize(scope, state, public)?;
                let private = materialize(scope, state, private)?;
                let pair = v8::Object::new(scope);
                put(scope, pair, "publicKey", public.into())?;
                put(scope, pair, "privateKey", private.into())?;
                return Ok(pair.into());
            }
            if name != "AES-CBC" {
                return Err(error("NotSupportedError"));
            }
            if usages.is_empty()
                || usages
                    .iter()
                    .any(|usage| matches!(usage, Usage::Sign | Usage::Verify))
            {
                return Err(error("SyntaxError"));
            }
            let parameters = dictionary(args.get(0))?;
            let length = property(scope, parameters, "length")?;
            if length.is_undefined() {
                return Err(error("TypeError"));
            }
            let length = length
                .number_value(scope)
                .ok_or(V8BindingError::JavaScriptException)?
                .trunc();
            if !length.is_finite() || length < 0.0 || length > 65535.0 {
                return Err(error("TypeError"));
            }
            let key = Key::generate(length as usize, args.get(1).boolean_value(scope), usages)
                .map_err(error)?;
            Ok(materialize(scope, state, key)?.into())
        }
        "importKey" => {
            let format = text(scope, args.get(0))?;
            let data = snapshot_key_data(scope, &format, args.get(1))?;
            let usages = usages(scope, args.get(4))?;
            let algorithm = algorithm(scope, args.get(2))?;
            let key = import(
                scope,
                &format,
                data,
                &algorithm,
                args.get(3).boolean_value(scope),
                usages,
            )?;
            Ok(materialize(scope, state, key)?.into())
        }
        "exportKey" => {
            let format = text(scope, args.get(0))?;
            let key = native_key(scope, args.get(1))?;
            export(scope, &format, &key)
        }
        "encrypt" | "decrypt" => {
            let key = native_key(scope, args.get(1))?;
            let input = bytes(scope, args.get(2))?;
            let parameters = iv(scope, args.get(0))?;
            let decrypt = member == "decrypt";
            let result = key
                .crypt(
                    &parameters,
                    &input,
                    decrypt,
                    if decrypt {
                        Usage::Decrypt
                    } else {
                        Usage::Encrypt
                    },
                )
                .map_err(error)?;
            Ok(array_buffer_from_bytes(scope, &result).into())
        }
        "digest" => {
            let input = bytes(scope, args.get(1))?;
            let algorithm = algorithm(scope, args.get(0))?;
            let result = crate::crypto_key::digest(&algorithm, &input).map_err(error)?;
            Ok(array_buffer_from_bytes(scope, &result).into())
        }
        "wrapKey" => {
            let format = text(scope, args.get(0))?;
            let key = native_key(scope, args.get(1))?;
            let wrapping = native_key(scope, args.get(2))?;
            let parameters = iv(scope, args.get(3))?;
            let input = exported_bytes(&format, &key)?;
            let result = wrapping
                .crypt(&parameters, &input, false, Usage::WrapKey)
                .map_err(error)?;
            Ok(array_buffer_from_bytes(scope, &result).into())
        }
        "unwrapKey" => {
            let format = text(scope, args.get(0))?;
            let input = bytes(scope, args.get(1))?;
            let key = native_key(scope, args.get(2))?;
            let usages = usages(scope, args.get(6))?;
            let parameters = iv(scope, args.get(3))?;
            let algorithm = algorithm(scope, args.get(4))?;
            let decoded = Zeroizing::new(
                key.crypt(&parameters, &input, true, Usage::UnwrapKey)
                    .map_err(error)?,
            );
            let data = if format == "jwk" {
                let text = std::str::from_utf8(&decoded).map_err(|_| error("DataError"))?;
                let text = v8::String::new(scope, text)
                    .ok_or(V8BindingError::Allocation("unwrapped JWK"))?;
                v8::json::parse(scope, text).ok_or_else(|| error("DataError"))?
            } else {
                array_buffer_from_bytes(scope, &decoded).into()
            };
            let result = import(
                scope,
                &format,
                data,
                &algorithm,
                args.get(5).boolean_value(scope),
                usages,
            )?;
            Ok(materialize(scope, state, result)?.into())
        }
        "sign" | "verify" => {
            let key = native_key(scope, args.get(1))?;
            let first = bytes(scope, args.get(2))?;
            let data = if member == "verify" {
                Some(bytes(scope, args.get(3))?)
            } else {
                None
            };
            let name = algorithm(scope, args.get(0))?;
            if name != "Ed25519" {
                return Err(error("NotSupportedError"));
            }
            if member == "sign" {
                let signature = key.sign(&first).map_err(error)?;
                Ok(array_buffer_from_bytes(scope, &signature).into())
            } else {
                Ok(v8::Boolean::new(
                    scope,
                    key.verify(&first, data.as_ref().unwrap()).map_err(error)?,
                )
                .into())
            }
        }
        "deriveBits" | "deriveKey" => Err(error("NotSupportedError")),
        _ => Err(error("TypeError")),
    }
}
fn settle(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
    let Ok(data) = v8::Local::<v8::Array>::try_from(args.data()) else {
        return;
    };
    let Some(resolver) = data.get_index(scope, 0) else {
        return;
    };
    // This callback and its null-prototype data array are never exposed to script.
    // Slot zero is populated only with PromiseResolver::new below. V8 does not
    // expose a checked Value-to-PromiseResolver conversion.
    let resolver = unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(resolver) };
    let Some(value) = data.get_index(scope, 1) else {
        return;
    };
    let failed = data
        .get_index(scope, 2)
        .is_some_and(|value| value.is_true());
    if failed {
        resolver.reject(scope, value);
    } else {
        resolver.resolve(scope, value);
    }
}
pub(super) fn operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    object: v8::Local<'s, v8::Object>,
    interface: &str,
    member: &str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    if interface == "Crypto" {
        let id = stateful_id(scope, object).map_err(|_| error("TypeError"))?;
        if !matches!(
            &*platform_object(state, id)?,
            PlatformObjectRecord::Crypto { .. }
        ) {
            return Err(error("TypeError"));
        }
        return match member {
            "getRandomValues" => random(scope, args.get(0)),
            "randomUUID" => {
                if !state.secure_context.get() {
                    return Err(error("SecurityError"));
                }
                let mut bytes = [0; 16];
                getrandom::fill(&mut bytes).map_err(|_| error("OperationError"))?;
                bytes[6] = (bytes[6] & 15) | 64;
                bytes[8] = (bytes[8] & 63) | 128;
                let hex = bytes
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                v8_string(
                    scope,
                    &format!(
                        "{}-{}-{}-{}-{}",
                        &hex[..8],
                        &hex[8..12],
                        &hex[12..16],
                        &hex[16..20],
                        &hex[20..]
                    ),
                )
            }
            _ => Err(error("TypeError")),
        };
    }
    v8::tc_scope!(let tc,scope);
    let result = (|| {
        let id = stateful_id(tc, object).map_err(|_| error("TypeError"))?;
        if !matches!(
            &*platform_object(state, id).map_err(|_| error("TypeError"))?,
            PlatformObjectRecord::SubtleCrypto
        ) {
            return Err(error("TypeError"));
        }
        if !state.secure_context.get() {
            return Err(error("SecurityError"));
        }
        compute(tc, state, member, args)
    })();
    let exception = tc.exception();
    if tc.is_execution_terminating() {
        return Err(V8BindingError::JavaScriptException);
    }
    tc.reset();
    let (failed, value) = match result {
        Ok(value) => (false, value),
        Err(error) => {
            let value = match exception {
                Some(value) => value,
                None => dom_exception_value(
                    tc,
                    match &error {
                        V8BindingError::Unsupported { member, .. } => member.as_str(),
                        _ => "OperationError",
                    },
                )?,
            };
            (true, value)
        }
    };
    let resolver =
        v8::PromiseResolver::new(tc).ok_or(V8BindingError::Allocation("crypto promise"))?;
    let promise = resolver.get_promise(tc);
    let data = v8::Array::new(tc, 3);
    let null = v8::null(tc);
    if data.set_prototype(tc, null.into()) != Some(true) {
        return Err(V8BindingError::Allocation("crypto settlement prototype"));
    }
    if data.set_index(tc, 0, resolver.into()) != Some(true)
        || data.set_index(tc, 1, value) != Some(true)
    {
        return Err(V8BindingError::Allocation("crypto settlement data"));
    }
    let failed = v8::Boolean::new(tc, failed);
    if data.set_index(tc, 2, failed.into()) != Some(true) {
        return Err(V8BindingError::Allocation("crypto settlement kind"));
    }
    let callback = v8::Function::builder(settle)
        .data(data.into())
        .build(tc)
        .ok_or(V8BindingError::Allocation("crypto settlement"))?;
    tc.enqueue_microtask(callback);
    Ok(promise.into())
}
fn random<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    if !(value.is_int8_array()
        || value.is_uint8_array()
        || value.is_uint8_clamped_array()
        || value.is_int16_array()
        || value.is_uint16_array()
        || value.is_int32_array()
        || value.is_uint32_array()
        || value.is_big_int64_array()
        || value.is_big_uint64_array())
    {
        return Err(error("TypeMismatchError"));
    }
    let view = v8::Local::<v8::ArrayBufferView>::try_from(value)
        .map_err(|_| error("TypeMismatchError"))?;
    if view.byte_length() > 65536 {
        return Err(error("QuotaExceededError"));
    }
    let buffer = view.buffer(scope).ok_or_else(|| error("TypeError"))?;
    if buffer.was_detached()
        || buffer.get_backing_store().is_shared()
        || buffer.get_backing_store().is_resizable_by_user_javascript()
    {
        return Err(error("TypeError"));
    }
    let mut bytes = Zeroizing::new(vec![0; view.byte_length()]);
    getrandom::fill(&mut bytes).map_err(|_| error("OperationError"))?;
    let store = buffer.get_backing_store();
    let start = view.byte_offset();
    if start
        .checked_add(bytes.len())
        .is_none_or(|end| end > store.byte_length())
    {
        return Err(error("TypeError"));
    }
    for (cell, byte) in store[start..start + bytes.len()].iter().zip(bytes.iter()) {
        cell.set(*byte);
    }
    Ok(value)
}
fn crypto_getter(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let Some(state) = binding_state_for_receiver(scope, args.this()) else {
        throw_type_error(scope, "Crypto global receiver");
        return;
    };
    if !state
        .global_object
        .to_local(scope)
        .is_some_and(|global| global.strict_equals(args.this().into()))
    {
        throw_type_error(scope, "Crypto global receiver");
        return;
    }
    let id = state
        .platform_objects
        .borrow()
        .iter()
        .find_map(|(&id, record)| {
            matches!(record, PlatformObjectRecord::Crypto { .. }).then_some(id)
        });
    let Some(id) = id else {
        return;
    };
    match new_platform_object(scope, &state, "Crypto", id) {
        Ok(object) => rv.set(object.into()),
        Err(error) => throw_binding_error(scope, &error),
    }
}
pub(super) fn install(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    context: v8::Local<v8::Context>,
    constructors: &BTreeMap<&'static str, v8::Global<v8::Function>>,
) -> Result<(), V8BindingError> {
    let subtle = register_platform_object(state, PlatformObjectRecord::SubtleCrypto);
    let crypto =
        register_platform_object(state, PlatformObjectRecord::Crypto { subtle_id: subtle });
    let _ = crypto;
    let getter = v8::Function::builder(crypto_getter)
        .build(scope)
        .ok_or(V8BindingError::Allocation("Crypto getter"))?;
    let undefined = v8::undefined(scope);
    let mut descriptor = v8::PropertyDescriptor::new_from_get_set(getter.into(), undefined.into());
    descriptor.set_enumerable(true);
    descriptor.set_configurable(true);
    let key =
        v8::String::new(scope, "crypto").ok_or(V8BindingError::Allocation("Crypto global"))?;
    if context
        .global(scope)
        .define_property(scope, key.into(), &descriptor)
        != Some(true)
    {
        return Err(V8BindingError::Allocation("Crypto global"));
    }
    if !state.secure_context.get() {
        for name in ["CryptoKey", "SubtleCrypto"] {
            let key = v8::String::new(scope, name)
                .ok_or(V8BindingError::Allocation("secure crypto constructor"))?;
            context.global(scope).delete(scope, key.into());
        }
        let constructor = constructors.get("Crypto").cloned();
        if let Some(constructor) = constructor {
            let constructor = v8::Local::new(scope, &constructor);
            let prototype = property(scope, constructor.into(), "prototype")?;
            let prototype = dictionary(prototype)?;
            for name in ["subtle", "randomUUID"] {
                let key = v8::String::new(scope, name)
                    .ok_or(V8BindingError::Allocation("secure crypto member"))?;
                prototype.delete(scope, key.into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn realm<'s>(
        scope: &mut v8::PinScope<'s, '_, ()>,
        id: u32,
        secure: bool,
        worker: bool,
    ) -> (v8::Local<'s, v8::Context>, Rc<V8BindingState>) {
        let context = v8::Context::new(scope, Default::default());
        context.set_slot(Rc::new(crate::realm::SecureContextState(secure)));
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let document = dom.borrow().document();
        let state = {
            let scope = &mut v8::ContextScope::new(scope, context);
            install_generated_bindings_with_root_for_global(
                scope,
                context,
                dom,
                RealmId(id),
                1,
                document,
                if worker {
                    BindingGlobal::Worker
                } else {
                    BindingGlobal::Window
                },
            )
            .unwrap()
        };
        (context, state)
    }
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn native_aes_operations_ignore_mutated_key_descriptors_and_preserve_export_policy() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let (context, _state) = realm(scope, 0, true, false);
        let scope = &mut v8::ContextScope::new(scope, context);
        eval(
            scope,
            r#"globalThis.done=false;globalThis.failure=null;(async()=>{const key=await crypto.subtle.generateKey({name:'aEs-CbC',length:128},false,['encrypt','decrypt']);globalThis.key=key;if(!(key instanceof CryptoKey)||key.type!=='secret'||key.extractable||!Object.isFrozen(key.usages))throw Error('key metadata');key.algorithm.name='RSA-OAEP';key.algorithm.length=8;Object.defineProperty(key,'extractable',{value:true});const iv=new Uint8Array(16),plain=new Uint8Array([1,2,3]);const cipher=await crypto.subtle.encrypt({name:'AES-CBC',iv},key,plain);const clone=structuredClone({a:key,b:key});if(clone.a===key||clone.a!==clone.b||clone.a.algorithm.name!=='AES-CBC'||clone.a.extractable)throw Error('native clone slots');const restored=await crypto.subtle.decrypt({name:'AES-CBC',iv},clone.a,cipher);if(String(new Uint8Array(restored))!=='1,2,3')throw Error('roundtrip');let denied=false;try{await crypto.subtle.exportKey('raw',key)}catch(e){denied=e.name==='InvalidAccessError'}if(!denied)throw Error('extractability bypass');let transfer=false;try{structuredClone(key,{transfer:[key]})}catch(e){transfer=e.name==='DataCloneError'}if(!transfer)throw Error('CryptoKey transferred');const digest=await crypto.subtle.digest('SHA-256',new TextEncoder().encode('abc'));if(new Uint8Array(digest)[0]!==186)throw Error('digest');done=true})().catch(e=>failure=String(e.stack||e))"#,
        );
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "done&&failure===null").is_true());
    }
    #[test]
    fn ed25519_core_helper_jwk_sign_verify_wrap_and_nonextractable_key_pair() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let (context, _state) = realm(scope, 0, true, false);
        let scope = &mut v8::ContextScope::new(scope, context);
        eval(
            scope,
            r#"globalThis.done=false;globalThis.failure=null;(async()=>{const jwk={kty:'OKP',crv:'Ed25519',d:'n4Ni-HpISpVObnQMW0wOhCKROaIKqKtW_2ZYb2p9KcU',x:'JrQLj5P_89iXES9-vFgrIy29clF9CC_oPPsw3c5D0bs'};const privateKey=await crypto.subtle.importKey('jwk',jwk,'Ed25519',true,['sign']);const publicKey=await crypto.subtle.importKey('jwk',{kty:jwk.kty,crv:jwk.crv,x:jwk.x},'Ed25519',true,['verify']);const bytes=new TextEncoder().encode('signature base');const signature=await crypto.subtle.sign('Ed25519',privateKey,bytes);if(!await crypto.subtle.verify('Ed25519',publicKey,signature,bytes))throw Error('signature');const pair=await crypto.subtle.generateKey('Ed25519',false,['sign','verify']);if(pair.privateKey.extractable||!pair.publicKey.extractable)throw Error('pair extractability');const wrapper=await crypto.subtle.generateKey({name:'AES-CBC',length:256},false,['wrapKey','unwrapKey']);const iv=new Uint8Array(16);const wrapped=await crypto.subtle.wrapKey('jwk',privateKey,wrapper,{name:'AES-CBC',iv});const unwrapped=await crypto.subtle.unwrapKey('jwk',wrapped,wrapper,{name:'AES-CBC',iv},'Ed25519',false,['sign']);if(!await crypto.subtle.verify('Ed25519',publicKey,await crypto.subtle.sign('Ed25519',unwrapped,bytes),bytes))throw Error('wrapped signature');done=true})().catch(e=>failure=String(e.stack||e))"#,
        );
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "done&&failure===null").is_true());
    }
    #[test]
    fn secure_wire_clone_to_worker_and_insecure_receiver_messageerror() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let (source, _source_state) = realm(scope, 0, true, false);
        let wire = {
            let scope = &mut v8::ContextScope::new(scope, source);
            eval(
                scope,
                "crypto.subtle.generateKey({name:'AES-CBC',length:128},false,['encrypt','decrypt']).then(key=>globalThis.key=key)",
            );
            scope.perform_microtask_checkpoint();
            let key = eval(scope, "key");
            structured_clone::serialize_wire(scope, key, &[]).unwrap()
        };
        assert!(
            wire.hosts
                .iter()
                .any(|host| matches!(host, crate::message_wire::WireHost::CryptoKey(_)))
        );
        let (worker, _worker_state) = realm(scope, 1, true, true);
        {
            let scope = &mut v8::ContextScope::new(scope, worker);
            let (key, ports) = structured_clone::deserialize_wire(scope, wire.clone()).unwrap();
            assert!(ports.is_empty());
            let native = native_key(scope, key).unwrap();
            assert!(!native.extractable());
            assert_eq!(native.algorithm(), "AES-CBC");
            assert!(native.permits(Usage::Encrypt));
            let global = scope.get_current_context().global(scope);
            let name = v8::String::new(scope, "received").unwrap();
            global.set(scope, name.into(), key);
            assert!(
                eval(
                    scope,
                    "received instanceof CryptoKey&&received.algorithm.length===128"
                )
                .is_true()
            );
            eval(
                scope,
                "globalThis.roundtrip=false;(async()=>{const iv=new Uint8Array(16);const cipher=await crypto.subtle.encrypt({name:'AES-CBC',iv},received,new Uint8Array([4,5,6]));const plain=await crypto.subtle.decrypt({name:'AES-CBC',iv},received,cipher);roundtrip=String(new Uint8Array(plain))==='4,5,6'})()",
            );
            scope.perform_microtask_checkpoint();
            assert!(eval(scope, "roundtrip").is_true());
        }
        let (insecure, state) = realm(scope, 2, false, false);
        {
            let scope = &mut v8::ContextScope::new(scope, insecure);
            assert!(eval(scope,"typeof CryptoKey==='undefined'&&typeof SubtleCrypto==='undefined'&&!('subtle' in crypto)&&!('randomUUID' in crypto)").is_true());
            eval(
                scope,
                "globalThis.messages=0;globalThis.errors=0;addEventListener('message',()=>messages++);addEventListener('messageerror',e=>{if(e.isTrusted&&e.data===null&&e.ports.length===0&&e.origin==='https://source.test')errors++})",
            );
            let mut delivery =
                message_delivery::Delivery::wire(&state, wire, "https://source.test".into());
            assert!(
                delivery
                    .resume(scope, &state, global_events::GLOBAL_TARGET)
                    .unwrap()
            );
            assert!(eval(scope, "messages===0&&errors===1").is_true());
        }
    }
    #[test]
    fn random_integer_views_boundaries_and_original_algorithm_getter_exception() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let (context, _state) = realm(scope, 0, true, false);
        let scope = &mut v8::ContextScope::new(scope, context);
        eval(
            scope,
            r#"const bytes=new Uint8Array(20).fill(99),view=new Uint16Array(bytes.buffer,4,4);if(crypto.getRandomValues(view)!==view||bytes[0]!==99||bytes[19]!==99)throw Error('random view');let mismatch=false;try{crypto.getRandomValues(new Float32Array(2))}catch(e){mismatch=e.name==='TypeMismatchError'}if(!mismatch)throw Error('random type');let quota=false;try{crypto.getRandomValues(new Uint8Array(65537))}catch(e){quota=e.name==='QuotaExceededError'}if(!quota)throw Error('random bound');globalThis.same=false;const sentinel={};crypto.subtle.digest({get name(){throw sentinel}},new Uint8Array()).catch(e=>same=e===sentinel);globalThis.snapshot=false;const input=new TextEncoder().encode('abc');crypto.subtle.digest({get name(){input.fill(0);return 'SHA-256'}},input).then(value=>snapshot=new Uint8Array(value)[0]===186);if(!/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(crypto.randomUUID()))throw Error('UUID')"#,
        );
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "same&&snapshot").is_true());
    }
}
