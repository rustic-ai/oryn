//! Native Error/DOMException branding and private serialized stack snapshots.
use super::*;
fn key<'s>(scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Private>> {
    let name = v8::String::new(scope, "oryn.error.stack")?;
    Some(v8::Private::for_api(scope, Some(name)))
}
fn is_dom(scope: &mut v8::PinScope, value: v8::Local<v8::Value>) -> bool {
    let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
        return false;
    };
    let Some(state) = binding_state_for_receiver(scope, object) else {
        return false;
    };
    let Ok(id) = stateful_id(scope, object) else {
        return false;
    };
    let wrapper_matches = state
        .platform_wrappers
        .borrow()
        .get(&id)
        .and_then(|wrapper| wrapper.to_local(scope))
        .is_some_and(|wrapper| wrapper.strict_equals(value));
    let branded = matches!(
        state.platform_objects.borrow().get(&id),
        Some(PlatformObjectRecord::DomException { .. })
    );
    wrapper_matches && branded
}
fn is_error(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let value = args.get(0);
    rv.set_bool(value.is_native_error() || is_dom(scope, value));
}
fn get_stack(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let object = args.this();
    let Some(key) = key(scope) else { return };
    if object.has_private(scope, key) == Some(true) {
        if let Some(value) = object.get_private(scope, key) {
            rv.set(value)
        }
        return;
    }
    if object.is_native_error()
        && let Ok(getter) = v8::Local::<v8::Function>::try_from(args.data())
    {
        if let Some(value) = getter.call(scope, object.into(), &[]) {
            rv.set(value)
        }
        return;
    }
    throw_type_error(scope, "Error stack receiver required");
}
fn set_stack(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
    let object = args.this();
    if !object.is_native_error() && !is_dom(scope, object.into()) {
        // An inherited setter must permit ordinary Error-style subclasses such
        // as WPT's Object.create(Error.prototype) AssertionError. Store an own
        // public data property; this does not grant the native/private brand.
        let Some(name) = v8::String::new(scope, "stack") else {
            return;
        };
        if object.create_data_property(scope, name.into(), args.get(0)) == Some(false) {
            throw_type_error(scope, "Cannot define Error stack on this object");
        }
        return;
    }
    if let Some(key) = key(scope) {
        object.set_private(scope, key, args.get(0));
    }
}
pub(super) fn set(
    scope: &mut v8::PinScope,
    object: v8::Local<v8::Object>,
    units: &[u16],
) -> Option<()> {
    let value = v8::String::new_from_two_byte(scope, units, v8::NewStringType::Normal)?;
    let key = key(scope)?;
    object.set_private(scope, key, value.into())?;
    Some(())
}
pub(super) fn stack(scope: &mut v8::PinScope, object: v8::Local<v8::Object>) -> Option<Vec<u16>> {
    let key = key(scope)?;
    let value = object.get_private(scope, key)?;
    let value = v8::Local::<v8::String>::try_from(value).ok()?;
    let mut units = vec![0; value.length()];
    value.write_v2(scope, 0, &mut units, v8::WriteFlags::empty());
    Some(units)
}
pub(super) fn capture_dom(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    id: u64,
    object: v8::Local<v8::Object>,
) -> Option<()> {
    let (name, message) = match state.platform_objects.borrow().get(&id) {
        Some(PlatformObjectRecord::DomException { name, message, .. }) => {
            (name.clone(), message.clone())
        }
        _ => return Some(()),
    };
    let mut units = Vec::new();
    for text in [
        name.as_str(),
        if message.is_empty() { "" } else { ": " },
        message.as_str(),
    ] {
        units.extend(text.encode_utf16());
    }
    if let Some(trace) = v8::StackTrace::current_stack_trace(scope, 32) {
        for index in 0..trace.get_frame_count() {
            let frame = trace.get_frame(scope, index)?;
            let name = frame
                .get_function_name(scope)
                .map(|name| name.to_rust_string_lossy(scope))
                .unwrap_or_default();
            let source = frame
                .get_script_name_or_source_url(scope)
                .map(|name| name.to_rust_string_lossy(scope))
                .unwrap_or_default();
            units.extend(
                format!(
                    "\n    at {} ({}:{}:{})",
                    if name.is_empty() {
                        "<anonymous>"
                    } else {
                        &name
                    },
                    source,
                    frame.get_line_number(),
                    frame.get_column()
                )
                .encode_utf16(),
            );
        }
    }
    set(scope, object, &units)
}
pub(super) fn install(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
    prototype: v8::Local<v8::Object>,
) -> Result<(), V8BindingError> {
    let stack = v8::String::new(scope, "stack").ok_or(V8BindingError::Allocation("Error stack"))?;
    let empty = v8::String::new(scope, "").ok_or(V8BindingError::Allocation("Error sample"))?;
    let sample = v8::Exception::error(scope, empty);
    let sample = v8::Local::<v8::Object>::try_from(sample)
        .map_err(|_| V8BindingError::Allocation("Error sample"))?;
    let descriptor = sample
        .get_own_property_descriptor(scope, stack.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or(V8BindingError::Allocation("native stack descriptor"))?;
    let get = v8::String::new(scope, "get").ok_or(V8BindingError::Allocation("stack getter"))?;
    let native = descriptor
        .get(scope, get.into())
        .ok_or(V8BindingError::Allocation("native stack getter"))?;
    let getter = v8::Function::builder(get_stack)
        .data(native)
        .build(scope)
        .ok_or(V8BindingError::Allocation("Error stack getter"))?;
    let setter = v8::Function::new(scope, set_stack)
        .ok_or(V8BindingError::Allocation("Error stack setter"))?;
    let mut descriptor = v8::PropertyDescriptor::new_from_get_set(getter.into(), setter.into());
    descriptor.set_configurable(true);
    descriptor.set_enumerable(false);
    prototype
        .define_property(scope, stack.into(), &descriptor)
        .ok_or(V8BindingError::Allocation("Error stack accessor"))?;
    let error =
        v8::String::new(scope, "Error").ok_or(V8BindingError::Allocation("Error constructor"))?;
    let constructor = context
        .global(scope)
        .get(scope, error.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or(V8BindingError::Allocation("Error constructor"))?;
    let function = v8::Function::builder(is_error)
        .length(1)
        .constructor_behavior(v8::ConstructorBehavior::Throw)
        .build(scope)
        .ok_or(V8BindingError::Allocation("Error.isError"))?;
    let name = v8::String::new(scope, "isError")
        .ok_or(V8BindingError::Allocation("Error.isError name"))?;
    constructor
        .define_own_property(
            scope,
            name.into(),
            function.into(),
            v8::PropertyAttribute::DONT_ENUM,
        )
        .ok_or(V8BindingError::Allocation("Error.isError property"))?;
    Ok(())
}
/// Walk only freshly deserialized own data properties and native Map/Set
/// entries. No getters from author objects are evaluated by this reconciliation.
pub(super) fn normalize_clone<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    root: v8::Local<'s, v8::Value>,
) -> Option<()> {
    let mut pending = vec![root];
    let mut seen: BTreeMap<i32, Vec<v8::Local<v8::Object>>> = BTreeMap::new();
    let stack = v8::String::new(scope, "stack")?;
    let value_key = v8::String::new(scope, "value")?;
    while let Some(value) = pending.pop() {
        let Ok(object) = v8::Local::<v8::Object>::try_from(value) else {
            continue;
        };
        let bucket = seen.entry(object.get_identity_hash().get()).or_default();
        if bucket.iter().any(|old| old.strict_equals(value)) {
            continue;
        }
        bucket.push(object);
        if object.is_native_error() {
            let own = object.has_own_property(scope, stack.into())?;
            if own {
                let value = object.get(scope, stack.into())?;
                let private = key(scope)?;
                object.set_private(scope, private, value)?;
                object.delete(scope, stack.into())?;
            }
        }
        if let Ok(map) = v8::Local::<v8::Map>::try_from(value) {
            let entries = map.as_array(scope);
            for index in 0..entries.length() {
                pending.push(entries.get_index(scope, index)?);
            }
        }
        if let Ok(set) = v8::Local::<v8::Set>::try_from(value) {
            let entries = set.as_array(scope);
            for index in 0..entries.length() {
                pending.push(entries.get_index(scope, index)?);
            }
        }
        if value.is_array_buffer_view() || value.is_array_buffer() || value.is_shared_array_buffer()
        {
            continue;
        }
        let keys = object.get_own_property_names(
            scope,
            v8::GetPropertyNamesArgs {
                property_filter: v8::PropertyFilter::ALL_PROPERTIES,
                key_conversion: v8::KeyConversionMode::ConvertToString,
                ..Default::default()
            },
        )?;
        for index in 0..keys.length() {
            let name = keys.get_index(scope, index)?;
            let name = v8::Local::<v8::Name>::try_from(name).ok()?;
            let descriptor = object.get_own_property_descriptor(scope, name)?;
            let Ok(descriptor) = v8::Local::<v8::Object>::try_from(descriptor) else {
                continue;
            };
            if descriptor.has_own_property(scope, value_key.into())? {
                pending.push(descriptor.get(scope, value_key.into())?);
            }
        }
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn native_error_clone_preserves_private_stack_causes_and_dom_exception_brand() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{
            const values=[new Error('a'),new TypeError('b'),new RangeError('c'),new DOMException('message','SyntaxError')];
            try{document.createElement('')}catch(e){values.push(e)}
            for(const error of values){const original=error.stack,copy=structuredClone(error);if(typeof original!=='string'||copy.stack!==original||Object.hasOwn(copy,'stack')||!Error.isError(copy))throw Error('stack copy');const again=structuredClone(copy);if(again.stack!==original)throw Error('second clone');}
            const cause=new TypeError('cause'),error=new Error('outer',{cause});const graph={error,map:new Map([[error,new Set([cause])]])};graph.self=graph;const copy=structuredClone(graph);
            if(copy.self!==copy||copy.error.stack!==error.stack||copy.error.cause.stack!==cause.stack||copy.map.keys().next().value!==copy.error||Object.hasOwn(copy.error.cause,'stack'))throw Error('nested graph');
            const fake=Object.create(DOMException.prototype);if(Error.isError(fake)||Error.isError(new Proxy(new Error(),{})))throw Error('false native brand');
            const marker={};Object.defineProperty(error,'name',{get(){throw marker}});try{structuredClone(error);throw Error('getter ignored')}catch(e){if(e!==marker)throw e}
            return true;
        })()"#).is_true());
    }
    #[test]
    fn lazy_native_exception_captures_stack_without_reading_public_constructor() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope,r#"(()=>{const NativeDOMException=DOMException;globalThis.DOMException=function(){throw Error('public constructor called')};let failure;try{document.createElement('')}catch(error){failure=error}if(!(failure instanceof NativeDOMException)||failure.name!=='InvalidCharacterError'||typeof failure.stack!=='string'||!failure.stack.includes('InvalidCharacterError')||Object.hasOwn(failure,'stack'))throw Error('native stack');const stack=failure.stack,copy=structuredClone(failure);if(copy.stack!==stack||copy.name!==failure.name||!Error.isError(copy))throw Error('native stack clone');const getter=Object.getOwnPropertyDescriptor(Error.prototype,'stack').get;let rejected=false;try{getter.call(Object.create(NativeDOMException.prototype))}catch(error){rejected=error instanceof TypeError}return rejected})()"#).is_true());
    }
    #[test]
    fn dom_exception_stack_survives_native_wire_without_reconstruction_at_receiver() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let original = eval(
            scope,
            "globalThis.original=new DOMException('wire','DataError');original",
        );
        let wire = structured_clone::serialize_wire(scope, original, &[]).unwrap();
        assert!(wire.hosts.iter().any(|host|matches!(host,crate::message_wire::WireHost::DomException{stack,..}if !stack.is_empty())));
        let (copy, _) = structured_clone::deserialize_wire(scope, wire).unwrap();
        let key = v8::String::new(scope, "copy").unwrap();
        context.global(scope).set(scope, key.into(), copy);
        assert!(eval(scope,"copy.stack===original.stack&&copy.name==='DataError'&&copy.message==='wire'&&!Object.hasOwn(copy,'stack')&&Error.isError(copy)").is_true());
    }
    #[test]
    fn wpt_assertion_error_keeps_failure_message_and_writable_ordinary_stack() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let _state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        assert!(eval(scope, r#"(()=>{
            // Pinned testharness shape: no native Error construction for this,
            // Error.prototype inheritance, then ordinary this.stack assignment.
            function AssertionError(message) {
                this.message=message;
                this.stack=(new Error(message)).stack;
            }
            AssertionError.prototype=Object.create(Error.prototype);
            AssertionError.prototype.name='AssertionError';
            function assert_equals(actual,expected) {
                if(actual!==expected)throw new AssertionError('expected '+expected+' but got '+actual);
            }
            let caught; try { assert_equals('actual','expected'); } catch(error) { caught=error; }
            if(!(caught instanceof AssertionError)||caught.message!=='expected expected but got actual')throw Error('assertion failure was masked');
            if(typeof caught.stack!=='string'||!caught.stack.includes('expected expected but got actual'))throw Error('assertion stack missing');
            const d=Object.getOwnPropertyDescriptor(caught,'stack');
            if(!d||!d.writable||!d.enumerable||!d.configurable||d.value!==caught.stack)throw Error('ordinary stack descriptor');
            caught.stack='rewritten assertion stack';
            if(caught.stack!=='rewritten assertion stack'||Error.isError(caught))throw Error('ordinary assignment granted native brand');
            const copy=structuredClone(caught);
            if(Error.isError(copy)||copy.message!==caught.message||copy.stack!==caught.stack)throw Error('ordinary clone changed brand');
            const native=structuredClone(new TypeError('native'));
            native.stack='native replacement';
            if(!Error.isError(native)||Object.hasOwn(native,'stack')||structuredClone(native).stack!=='native replacement')throw Error('native private stack changed');
            const dom=new DOMException('dom','DataError'); dom.stack='dom replacement';
            if(!Error.isError(dom)||Object.hasOwn(dom,'stack')||structuredClone(dom).stack!=='dom replacement')throw Error('DOMException private stack changed');
            const getter=Object.getOwnPropertyDescriptor(Error.prototype,'stack').get;
            const setter=Object.getOwnPropertyDescriptor(Error.prototype,'stack').set;
            for(const fake of [Object.create(Error.prototype),Object.create(DOMException.prototype),new Proxy(new Error(),{})]) {
                setter.call(fake,'public stack');
                if(Error.isError(fake))throw Error('forged brand');
                try { getter.call(fake); throw Error('forged private stack accepted'); } catch(e) { if(!(e instanceof TypeError))throw e; }
            }
            try { setter.call(Object.preventExtensions(Object.create(Error.prototype)),'x'); throw Error('nonextensible assignment accepted'); } catch(e) { if(!(e instanceof TypeError))throw e; }
            const marker={};
            try { setter.call(new Proxy({}, {defineProperty(){throw marker}}),'x'); throw Error('proxy exception swallowed'); } catch(e) { if(e!==marker)throw e; }
            return true;
        })()"#).is_true());
    }
    #[test]
    fn ordinary_stack_assignment_does_not_brand_cross_realm_objects() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let _first = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body>").into_runtime().unwrap(),
            ));
            install_generated_bindings(scope, first, dom, RealmId(1), 1).unwrap()
        };
        let second = v8::Context::new(scope, Default::default());
        let _second = {
            let scope = &mut v8::ContextScope::new(scope, second);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body>").into_runtime().unwrap(),
            ));
            let state = install_generated_bindings(scope, second, dom, RealmId(2), 1).unwrap();
            eval(
                scope,
                "globalThis.native=new DOMException('other','DataError');globalThis.fake=Object.create(Error.prototype)",
            );
            state
        };
        let scope = &mut v8::ContextScope::new(scope, first);
        // Model same-origin page realms; standalone contexts have distinct
        // opaque access tokens by default. Do not disable native access checks.
        let token = v8::String::new(scope, "https://same-origin.test").unwrap();
        first.set_security_token(token.into());
        second.set_security_token(token.into());
        let other = second.global(scope);
        let name = v8::String::new(scope, "other").unwrap();
        first.global(scope).set(scope, name.into(), other.into());
        assert!(eval(scope, r#"(()=>{
            const setter=Object.getOwnPropertyDescriptor(Error.prototype,'stack').set;
            setter.call(other.fake,'foreign ordinary');setter.call(other.native,'foreign native');
            if(Error.isError(other.fake)||other.Error.isError(other.fake)||other.fake.stack!=='foreign ordinary')throw Error('foreign fake brand');
            if(!Error.isError(other.native)||!other.Error.isError(other.native)||other.native.stack!=='foreign native'||Object.hasOwn(other.native,'stack'))throw Error('foreign native brand');
            return structuredClone(other.native).stack==='foreign native';
        })()"#).is_true());
    }
}
