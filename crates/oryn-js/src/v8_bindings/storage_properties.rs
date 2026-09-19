//! WebIDL legacy named properties backed by the same native Storage owner as methods.
use super::*;
// Match the default native context-store quota. Account encoded key/value bytes
// before publishing a mutation, including replacement rather than double-counting it.
const FALLBACK_AREA_BYTES: usize = 10 * 1024 * 1024;
pub(super) fn fallback_set(
    area: &mut BTreeMap<String, String>,
    key: String,
    value: String,
) -> Result<(), V8BindingError> {
    fallback_set_limited(area, key, value, FALLBACK_AREA_BYTES)
}
pub(super) fn validate_fallback_area(
    area: &BTreeMap<String, String>,
) -> Result<(), V8BindingError> {
    let total = area.iter().try_fold(0usize, |bytes, (key, value)| {
        bytes.checked_add(key.len())?.checked_add(value.len())
    });
    if total.is_none_or(|bytes| bytes > FALLBACK_AREA_BYTES) {
        return Err(V8BindingError::Unsupported {
            interface: "Storage".into(),
            member: "QuotaExceededError".into(),
        });
    }
    Ok(())
}
fn fallback_set_limited(
    area: &mut BTreeMap<String, String>,
    key: String,
    value: String,
    limit: usize,
) -> Result<(), V8BindingError> {
    if area.get(&key) == Some(&value) {
        return Ok(());
    }
    let total = area
        .iter()
        .filter(|(existing, _)| *existing != &key)
        .try_fold(0usize, |bytes, (key, value)| {
            bytes.checked_add(key.len())?.checked_add(value.len())
        })
        .and_then(|bytes| bytes.checked_add(key.len()))
        .and_then(|bytes| bytes.checked_add(value.len()));
    if total.is_none_or(|bytes| bytes > limit) {
        return Err(V8BindingError::Unsupported {
            interface: "Storage".into(),
            member: "QuotaExceededError".into(),
        });
    }
    area.insert(key, value);
    Ok(())
}
struct PrototypeIntrinsic(v8::Weak<v8::Function>);
pub(super) fn install_intrinsics<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Context>,
) -> Result<(), V8BindingError> {
    if context.get_slot::<PrototypeIntrinsic>().is_some() {
        return Ok(());
    }
    let name =
        v8::String::new(scope, "Reflect").ok_or(V8BindingError::Allocation("Storage intrinsic"))?;
    let global = context.global(scope);
    let reflect = global
        .get(scope, name.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or(V8BindingError::Allocation("Storage Reflect"))?;
    let name = v8::String::new(scope, "getPrototypeOf")
        .ok_or(V8BindingError::Allocation("Storage intrinsic"))?;
    let function = reflect
        .get(scope, name.into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .ok_or(V8BindingError::Allocation("Storage prototype intrinsic"))?;
    let retained = context_intrinsics::retain_function_in_context(
        scope,
        context,
        "storage-get-prototype-of",
        function,
    )?;
    context.set_slot(Rc::new(PrototypeIntrinsic(retained)));
    Ok(())
}
fn prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    // V8 Object::GetPrototype bypasses Proxy [[GetPrototypeOf]]. Retain the
    // realm's pristine intrinsic before author code rather than consult Reflect.
    let context = scope.get_current_context();
    let intrinsic = context
        .get_slot::<PrototypeIntrinsic>()
        .ok_or(V8BindingError::Allocation("Storage prototype intrinsic"))?;
    let function = intrinsic
        .0
        .to_local(scope)
        .ok_or(V8BindingError::Allocation("Storage prototype intrinsic"))?;
    let receiver = v8::undefined(scope);
    function
        .call(scope, receiver.into(), &[object.into()])
        .ok_or(V8BindingError::JavaScriptException)
}
fn key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: v8::Local<'s, v8::Name>,
) -> Result<Option<String>, V8BindingError> {
    v8::Local::<v8::String>::try_from(name)
        .ok()
        .map(|name| rust_string_preserving_utf16(scope, name))
        .transpose()
}
fn visible<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: v8::Local<'s, v8::Name>,
) -> Result<bool, V8BindingError> {
    // Skip this object's interceptor to avoid recursively querying the named
    // property being resolved. Prototype objects use ordinary [[GetOwnProperty]]
    // semantics, including Proxy traps and their abrupt completions.
    if object
        .has_real_named_property(scope, name)
        .ok_or(V8BindingError::JavaScriptException)?
    {
        return Ok(false);
    }
    let mut current = prototype(scope, object)?;
    let mut seen: Vec<v8::Local<v8::Object>> = vec![object];
    while let Ok(object) = v8::Local::<v8::Object>::try_from(current) {
        if seen.len() >= 1024 || seen.iter().any(|prior| prior.strict_equals(object.into())) {
            return Err(V8BindingError::Unsupported {
                interface: "Storage".into(),
                member: "RangeError".into(),
            });
        }
        seen.push(object);
        if object
            .has_own_property(scope, name)
            .ok_or(V8BindingError::JavaScriptException)?
        {
            return Ok(false);
        }
        current = prototype(scope, object)?;
    }
    Ok(true)
}

fn read<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: v8::Local<'s, v8::Name>,
) -> Result<Option<String>, V8BindingError> {
    let Some(key) = key(scope, name)? else {
        return Ok(None);
    };
    if !visible(scope, object, name)? {
        return Ok(None);
    }
    let state = storage_events::owner_state(scope, object)?;
    let index = storage_index(scope, object)?;
    if let Some(response) = storage_context_response(scope, &state, index, "get", Some(&key), None)?
    {
        return Ok(response
            .get("response")
            .and_then(|value| value.get("value"))
            .and_then(|value| value.as_str())
            .map(str::to_owned));
    }
    let value = state.storage_fallback.borrow()[index].get(&key).cloned();
    Ok(value)
}
fn write<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    key: &str,
    value: Option<v8::Local<'s, v8::Value>>,
) -> Result<(), V8BindingError> {
    let state = storage_events::owner_state(scope, object)?;
    let index = storage_index(scope, object)?;
    let value = value
        .map(|value| {
            value
                .to_string(scope)
                .ok_or(V8BindingError::JavaScriptException)
                .and_then(|value| rust_string_preserving_utf16(scope, value))
        })
        .transpose()?;
    // DOMString conversion can invoke script. Revalidate the original native
    // receiver after conversion before either backend or standalone mutation.
    let current = storage_events::owner_state(scope, object)?;
    if !Rc::ptr_eq(&state, &current) {
        return Err(V8BindingError::Unsupported {
            interface: "Storage".into(),
            member: "InvalidStateError".into(),
        });
    }
    if storage_context_response(
        scope,
        &state,
        index,
        if value.is_some() { "set" } else { "remove" },
        Some(key),
        value.as_deref(),
    )?
    .is_none()
    {
        let mut fallback = state.storage_fallback.borrow_mut();
        if let Some(value) = value {
            fallback_set(&mut fallback[index], key.into(), value)?;
        } else {
            fallback[index].remove(key);
        }
    }
    Ok(())
}
fn getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue,
) -> v8::Intercepted {
    match read(scope, args.holder(), name)
        .and_then(|value| value.map(|value| v8_string(scope, &value)).transpose())
    {
        Ok(Some(value)) => {
            rv.set(value);
            v8::Intercepted::kYes
        }
        Ok(None) => v8::Intercepted::kNo,
        Err(error) => {
            throw_binding_error(scope, &error);
            v8::Intercepted::kYes
        }
    }
}
fn setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: v8::Local<'s, v8::Name>,
    value: v8::Local<'s, v8::Value>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<()>,
) -> v8::Intercepted {
    let result = key(scope, name).and_then(|key| {
        key.map(|key| write(scope, args.holder(), &key, Some(value)))
            .transpose()
    });
    match result {
        Ok(None) => return v8::Intercepted::kNo,
        Ok(Some(())) => rv.set_bool(true),
        Err(error) => throw_binding_error(scope, &error),
    }
    v8::Intercepted::kYes
}
fn query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<v8::Integer>,
) -> v8::Intercepted {
    match read(scope, args.holder(), name) {
        Ok(Some(_)) => {
            rv.set_int32(0);
            v8::Intercepted::kYes
        }
        Ok(None) => v8::Intercepted::kNo,
        Err(error) => {
            throw_binding_error(scope, &error);
            v8::Intercepted::kYes
        }
    }
}
fn descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue,
) -> v8::Intercepted {
    match read(scope, args.holder(), name)
        .and_then(|value| value.map(|value| v8_string(scope, &value)).transpose())
    {
        Ok(Some(value)) => {
            let descriptor = v8::Object::new(scope);
            for (name, value) in [
                ("value", value),
                ("writable", v8::Boolean::new(scope, true).into()),
                ("enumerable", v8::Boolean::new(scope, true).into()),
                ("configurable", v8::Boolean::new(scope, true).into()),
            ] {
                let name = v8::String::new(scope, name).unwrap();
                descriptor.create_data_property(scope, name.into(), value);
            }
            rv.set(descriptor.into());
            v8::Intercepted::kYes
        }
        Ok(None) => v8::Intercepted::kNo,
        Err(error) => {
            throw_binding_error(scope, &error);
            v8::Intercepted::kYes
        }
    }
}
fn definer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: v8::Local<'s, v8::Name>,
    desc: &v8::PropertyDescriptor,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<()>,
) -> v8::Intercepted {
    if name.is_symbol() {
        return v8::Intercepted::kNo;
    }
    if desc.has_get() || desc.has_set() {
        rv.set_bool(false);
        if args.should_throw_on_error() {
            throw_type_error(scope, "Storage cannot define a named accessor");
        }
        return v8::Intercepted::kYes;
    }
    let value = if desc.has_value() {
        v8::Local::new(scope, desc.value())
    } else {
        v8::undefined(scope).into()
    };
    setter(scope, name, value, args, rv)
}
fn deleter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<v8::Boolean>,
) -> v8::Intercepted {
    // Hidden named properties are not own properties; deleting a prototype
    // shadow must leave the stored value intact until that shadow is removed.
    match read(scope, args.holder(), name) {
        Ok(None) => v8::Intercepted::kNo,
        Err(error) => {
            throw_binding_error(scope, &error);
            v8::Intercepted::kYes
        }
        Ok(Some(_)) => {
            match key(scope, name)
                .and_then(|key| write(scope, args.holder(), key.as_deref().unwrap(), None))
            {
                Ok(()) => rv.set_bool(true),
                Err(error) => throw_binding_error(scope, &error),
            }
            v8::Intercepted::kYes
        }
    }
}
fn index_name(name: &str) -> Option<u32> {
    name.parse::<u32>()
        .ok()
        .filter(|index| *index != u32::MAX && index.to_string() == name)
}
fn enumerate<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<v8::Array>,
    indexed: bool,
) {
    let result = (|| {
        let state = storage_events::owner_state(scope, args.holder())?;
        let index = storage_index(scope, args.holder())?;
        let entries = storage_entries(scope, &state, index)?;
        let mut names = Vec::new();
        for (name, _) in entries {
            let number = index_name(&name);
            if number.is_some() != indexed {
                continue;
            }
            let key = v8::Local::<v8::String>::try_from(v8_string(scope, &name)?)
                .map_err(|_| V8BindingError::Allocation("storage key"))?;
            if visible(scope, args.holder(), key.into())? {
                names.push(if let Some(index) = number {
                    v8::Integer::new_from_unsigned(scope, index).into()
                } else {
                    key.into()
                });
            }
        }
        Ok(v8::Array::new_with_elements(scope, &names))
    })();
    match result {
        Ok(value) => rv.set(value),
        Err(error) => throw_binding_error(scope, &error),
    }
}
fn enumerator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::PropertyCallbackArguments<'s>,
    rv: v8::ReturnValue<v8::Array>,
) {
    enumerate(scope, args, rv, false)
}
fn indexed_enumerator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::PropertyCallbackArguments<'s>,
    rv: v8::ReturnValue<v8::Array>,
) {
    enumerate(scope, args, rv, true)
}
fn indexed_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    rv: v8::ReturnValue,
) -> v8::Intercepted {
    let name = v8::String::new(scope, &index.to_string()).unwrap();
    getter(scope, name.into(), args, rv)
}
fn indexed_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    value: v8::Local<'s, v8::Value>,
    args: v8::PropertyCallbackArguments<'s>,
    rv: v8::ReturnValue<()>,
) -> v8::Intercepted {
    let name = v8::String::new(scope, &index.to_string()).unwrap();
    setter(scope, name.into(), value, args, rv)
}
fn indexed_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    rv: v8::ReturnValue<v8::Integer>,
) -> v8::Intercepted {
    let name = v8::String::new(scope, &index.to_string()).unwrap();
    query(scope, name.into(), args, rv)
}
fn indexed_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    rv: v8::ReturnValue,
) -> v8::Intercepted {
    let name = v8::String::new(scope, &index.to_string()).unwrap();
    descriptor(scope, name.into(), args, rv)
}
fn indexed_definer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    desc: &v8::PropertyDescriptor,
    args: v8::PropertyCallbackArguments<'s>,
    rv: v8::ReturnValue<()>,
) -> v8::Intercepted {
    let name = v8::String::new(scope, &index.to_string()).unwrap();
    definer(scope, name.into(), desc, args, rv)
}
fn indexed_deleter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    rv: v8::ReturnValue<v8::Boolean>,
) -> v8::Intercepted {
    let name = v8::String::new(scope, &index.to_string()).unwrap();
    deleter(scope, name.into(), args, rv)
}
pub(super) fn configure(instance: v8::Local<v8::ObjectTemplate>) {
    instance.set_named_property_handler(
        v8::NamedPropertyHandlerConfiguration::new()
            .getter(getter)
            .setter(setter)
            .query(query)
            .descriptor(descriptor)
            .enumerator(enumerator)
            .definer(definer)
            .deleter(deleter),
    );
    instance.set_indexed_property_handler(
        v8::IndexedPropertyHandlerConfiguration::new()
            .getter(indexed_getter)
            .setter(indexed_setter)
            .query(indexed_query)
            .descriptor(indexed_descriptor)
            .enumerator(indexed_enumerator)
            .definer(indexed_definer)
            .deleter(indexed_deleter),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, text: &str) -> v8::Local<'s, v8::Value> {
        let text = v8::String::new(scope, text).unwrap();
        v8::Script::compile(scope, text, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn fallback_quota_counts_replacements_and_encoded_bytes_atomically() {
        let mut area = BTreeMap::new();
        fallback_set_limited(&mut area, "a".into(), "1234".into(), 5).unwrap();
        fallback_set_limited(&mut area, "a".into(), "1234".into(), 5).unwrap();
        assert!(fallback_set_limited(&mut area, "b".into(), "".into(), 5).is_err());
        assert!(fallback_set_limited(&mut area, "a".into(), "12345".into(), 5).is_err());
        assert_eq!(area.get("a").map(String::as_str), Some("1234"));
        assert_eq!(area.len(), 1);
        fallback_set_limited(&mut area, "a".into(), "é".into(), 5).unwrap();
        fallback_set_limited(&mut area, "b".into(), "x".into(), 5).unwrap();
        assert!(fallback_set_limited(&mut area, "b".into(), "é".into(), 5).is_err());
        assert_eq!(area.get("b").map(String::as_str), Some("x"));
    }
    #[test]
    fn storage_method_and_named_quota_failures_preserve_values_and_area_independence() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<p>storage</p>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{const full='x'.repeat(10*1024*1024-1);for(const storage of [localStorage,sessionStorage]){storage.setItem('a',full);storage.a=full;for(const mutation of [()=>storage.setItem('b','x'),()=>{storage.b='x'},()=>Object.defineProperty(storage,'b',{value:'x'}),()=>storage.setItem('a',full+'x')]){try{mutation();return false}catch(error){if(error.name!=='QuotaExceededError')return false}if(storage.length!==1||storage.getItem('a')!==full||storage.getItem('b')!==null)return false;}}localStorage.removeItem('a');localStorage.b='small';return sessionStorage.a===full&&localStorage.b==='small'})()"#).is_true());
    }
    #[test]
    fn storage_snapshot_accepts_two_full_areas_and_counts_only_final_duplicate_value() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<p>storage</p>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let function = v8::Function::new(scope, restore_storage_callback).unwrap();
        let key = v8::String::new(scope, "restoreSnapshot").unwrap();
        let global = context.global(scope);
        global.create_data_property(scope, key.into(), function.into());
        assert!(eval(scope,r#"(()=>{const full='x'.repeat(10*1024*1024-1);restoreSnapshot([['a',full]],[['a',full]]);if(localStorage.a!==full||sessionStorage.a!==full)return false;restoreSnapshot([['a',full],['a','last']],[]);return localStorage.a==='last'&&localStorage.length===1&&sessionStorage.length===0})()"#).is_true());
    }
    #[test]
    fn storage_snapshot_quota_failure_does_not_publish_either_area() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<p>storage</p>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        // Direct native callback installation keeps the compatibility hook out of
        // public API assumptions and exercises its actual conversion/publication.
        let function = v8::Function::new(scope, restore_storage_callback).unwrap();
        let key = v8::String::new(scope, "restoreSnapshot").unwrap();
        let global = context.global(scope);
        global.create_data_property(scope, key.into(), function.into());
        assert!(eval(scope,r#"(()=>{localStorage.old='local';sessionStorage.old='session';try{restoreSnapshot([['new','new']],[['huge','é'.repeat(6*1024*1024)]]);return false}catch(error){return error.name==='QuotaExceededError'&&localStorage.old==='local'&&sessionStorage.old==='session'&&localStorage.length===1&&sessionStorage.length===1}})()"#).is_true());
    }
    #[test]
    fn storage_inherited_receivers_use_ordinary_assignment_without_mutating_the_area() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<p>storage</p>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{for(const storage of [localStorage,sessionStorage]){storage.clear();storage.x='original';const receiver=Object.create(storage);const value={toString(){throw Error('receiver conversion')}};receiver.x=value;if(receiver.x!==value||storage.getItem('x')!=='original')return false;if(!Reflect.set(storage,'other',value,receiver)||receiver.other!==value||storage.getItem('other')!==null)return false;Object.defineProperty(storage,'generic',{});if(storage.getItem('generic')!=='undefined')return false;}return true})()"#).is_true());
    }
    #[test]
    fn storage_visibility_preserves_proxy_traps_and_pristine_prototype_intrinsic() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<p>storage</p>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{const storage=localStorage;storage.x='stored';const original=Storage.prototype;const marker={};let mode='throw',calls=0;const proto=new Proxy({}, {getOwnPropertyDescriptor(target,key){if(key==='x'){calls++;if(mode==='throw')throw marker;if(mode==='hidden')return {value:'prototype',writable:true,enumerable:true,configurable:true};}return undefined;},getPrototypeOf(){return original}});Object.setPrototypeOf(storage,proto);Reflect.getPrototypeOf=()=>{throw Error('author replacement')};let caught;try{storage.x}catch(error){caught=error}if(caught!==marker||calls===0)return false;mode='visible';if(storage.x!=='stored')return false;mode='hidden';if(Object.getOwnPropertyDescriptor(storage,'x')!==undefined)return false;Object.setPrototypeOf(storage,original);return storage.x==='stored'})()"#).is_true());
    }
    #[test]
    fn storage_cyclic_proxy_prototype_is_bounded_and_retired_conversion_cannot_mutate() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<p>storage</p>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let function = v8::Function::new(
            scope,
            |scope: &mut v8::PinScope,
             _args: v8::FunctionCallbackArguments,
             _rv: v8::ReturnValue| {
                scope
                    .get_current_context()
                    .get_slot::<V8BindingState>()
                    .unwrap()
                    .retire_execution();
            },
        )
        .unwrap();
        let key = v8::String::new(scope, "retireStorageOwner").unwrap();
        let global = context.global(scope);
        global.create_data_property(scope, key.into(), function.into());
        assert!(eval(scope,r#"(()=>{const storage=localStorage,original=Storage.prototype;storage.x='old';let cyclic;cyclic=new Proxy({}, {getOwnPropertyDescriptor(){return undefined},getPrototypeOf(){return cyclic}});Object.setPrototypeOf(storage,cyclic);let bounded=false;try{storage.x}catch(error){bounded=error.name==='RangeError'}Object.setPrototypeOf(storage,original);if(!bounded)return false;try{storage.x={toString(){retireStorageOwner();return 'new'}};return false}catch(error){return error.name==='InvalidStateError'}})()"#).is_true());
        let state = context.get_slot::<V8BindingState>().unwrap();
        assert_eq!(
            state.storage_fallback.borrow()[0]
                .get("x")
                .map(String::as_str),
            Some("old")
        );
    }
    #[test]
    fn storage_named_properties_share_methods_and_preserve_prototype_visibility() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<p>storage</p>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"let correct=true;for(const storage of [localStorage,sessionStorage]){storage.clear();storage.named={toString(){return 'converted'}};storage[9]='numeric';storage['']='empty';Object.defineProperty(storage,'defined',{value:'first'});Object.defineProperty(storage,'defined',{value:'second'});const desc=Object.getOwnPropertyDescriptor(storage,'defined');correct&&=storage.getItem('named')==='converted'&&storage[9]==='numeric'&&storage.getItem('9')==='numeric'&&storage['']==='empty'&&desc.value==='second'&&desc.writable&&desc.enumerable&&desc.configurable;const keys=Object.keys(storage).sort().join(',');correct&&=keys===',9,defined,named';const symbol=Symbol();storage[symbol]='symbol';correct&&=storage[symbol]==='symbol'&&storage.length===4;delete storage[symbol];delete storage[9];correct&&=storage.getItem('9')===null&&!(9 in storage);Object.defineProperty(Storage.prototype,'hidden',{get(){return 'prototype'},set(){throw Error('prototype setter called')},configurable:true});storage.hidden='stored';correct&&=storage.hidden==='prototype'&&storage.getItem('hidden')==='stored'&&Object.getOwnPropertyDescriptor(storage,'hidden')===undefined;delete Storage.prototype.hidden;correct&&=storage.hidden==='stored';delete storage.hidden;correct&&=storage.getItem('hidden')===null;storage.clear='stored builtin';correct&&=storage.clear===Storage.prototype.clear&&storage.getItem('clear')==='stored builtin';storage.clear();}correct"#).is_true());
    }
    #[test]
    fn storage_named_conversion_and_descriptor_failures_preserve_existing_values() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<p>storage</p>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"localStorage.keyName='old';const marker={};let same=false;try{localStorage.keyName={toString(){localStorage.getItem('keyName');throw marker}}}catch(error){same=error===marker}let denied=false;try{Object.defineProperty(localStorage,'keyName',{get(){return 'fake'}})}catch(error){denied=error instanceof TypeError}let missing=0;for(const call of [()=>localStorage.key(),()=>localStorage.getItem(),()=>localStorage.setItem(),()=>localStorage.setItem('key'),()=>localStorage.removeItem()]){try{call()}catch(error){if(error instanceof TypeError)missing++}}same&&denied&&missing===5&&localStorage.keyName==='old'"#).is_true());
    }
}

#[cfg(test)]
mod intrinsic_retention_tests {
    use super::*;
    #[test]
    fn pristine_prototype_intrinsic_traces_its_context_without_native_root() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        let (context, intrinsic, anchor) = {
            v8::scope!(let handles, &mut isolate);
            let context = v8::Context::new(handles, Default::default());
            let scope = &mut v8::ContextScope::new(handles, context);
            install_intrinsics(scope, context).unwrap();
            let intrinsic = context.get_slot::<PrototypeIntrinsic>().unwrap().0.clone();
            let value = super::super::test_eval::eval(
                scope,
                r#"(()=>{
                Reflect.getPrototypeOf=()=>{throw Error('public replacement')};
                globalThis.trapCalls=0;const expected={marker:17};
                globalThis.expectedPrototype=expected;
                return new Proxy({}, {getPrototypeOf(){trapCalls++;return expected}});
            })()"#,
            )
            .try_cast::<v8::Object>()
            .unwrap();
            assert!(
                prototype(scope, value)
                    .unwrap()
                    .strict_equals(super::super::test_eval::eval(scope, "expectedPrototype"))
            );
            assert!(super::super::test_eval::eval(scope, "trapCalls===1").is_true());
            install_intrinsics(scope, context).unwrap();
            let same = context
                .get_slot::<PrototypeIntrinsic>()
                .unwrap()
                .0
                .to_local(scope)
                .unwrap();
            assert!(same.strict_equals(intrinsic.to_local(scope).unwrap().into()));
            (
                v8::Weak::new(scope, context),
                intrinsic,
                v8::Global::new(scope, value),
            )
        };
        isolate.low_memory_notification();
        isolate.low_memory_notification();
        {
            v8::scope!(let handles, &mut isolate);
            let context = context.to_local(handles).unwrap();
            let scope = &mut v8::ContextScope::new(handles, context);
            let value = v8::Local::new(scope, &anchor);
            assert!(
                prototype(scope, value)
                    .unwrap()
                    .strict_equals(super::super::test_eval::eval(scope, "expectedPrototype"))
            );
        }
        drop(anchor);
        isolate.low_memory_notification();
        isolate.low_memory_notification();
        v8::scope!(let handles, &mut isolate);
        assert!(context.to_local(handles).is_none());
        assert!(intrinsic.to_local(handles).is_none());
    }
}
