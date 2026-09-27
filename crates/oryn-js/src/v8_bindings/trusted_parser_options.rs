//! Parser authority is acquired once at stream creation, never from chunk text.
//! Native branded options retain their converted policy; public properties and
//! cross-realm numeric identities cannot manufacture parser authority.
use super::*;
#[derive(Default)]
struct Records(RefCell<BTreeMap<u64, (v8::Weak<v8::Object>, sanitizer::SinkOptions)>>);
fn invalid() -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "TrustedParserOptions".into(),
        member: "TypeError".into(),
    }
}
fn record(scope: &mut v8::PinScope, value: v8::Local<v8::Value>) -> Option<sanitizer::SinkOptions> {
    let object = value.try_cast::<v8::Object>().ok()?;
    if object.is_proxy() || object.internal_field_count() != 3 {
        return None;
    }
    let brand = object
        .get_internal_field(scope, 0)
        .and_then(|field| v8::Local::<v8::Value>::try_from(field).ok())?;
    if !brand.is_string() || brand.to_rust_string_lossy(scope) != "TrustedParserOptions" {
        return None;
    }
    let id = object
        .get_internal_field(scope, 1)
        .and_then(|field| v8::Local::<v8::Value>::try_from(field).ok())
        .and_then(exact_u64)?;
    let instance = object
        .get_internal_field(scope, 2)
        .and_then(|field| v8::Local::<v8::Value>::try_from(field).ok())
        .and_then(exact_u64)?;
    let context = object.get_creation_context(scope)?;
    let state = context.get_slot::<V8BindingState>()?;
    if state.instance_id != instance || state.execution_retired() {
        return None;
    }
    let records = context.get_slot::<Records>()?;
    let map = records.0.borrow();
    let (identity, options) = map.get(&id)?;
    let identity = identity.to_local(scope)?;
    if !identity.strict_equals(object.into()) {
        return None;
    }
    Some(options.clone())
}
fn call<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callback: v8::Weak<v8::Function>,
    input: &sanitizer::SinkOptions,
    extra: &[v8::Local<'s, v8::Value>],
) -> Result<Option<sanitizer::SinkOptions>, V8BindingError> {
    let callback = callback.to_local(scope).ok_or_else(invalid)?;
    let dictionary = input.stream_dictionary(scope)?;
    let mut arguments = vec![dictionary.into()];
    arguments.extend_from_slice(extra);
    let undefined = v8::undefined(scope).into();
    let result = callback
        .call(scope, undefined, &arguments)
        .ok_or(V8BindingError::JavaScriptException)?;
    if result.is_null_or_undefined() {
        return Ok(None);
    }
    sanitizer::options(scope, result, false, true).map(Some)
}
pub(super) fn acquire(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    value: v8::Local<v8::Value>,
    safe: bool,
    sink: &str,
) -> Result<sanitizer::SinkOptions, V8BindingError> {
    if !safe {
        if let Some(options) = record(scope, value) {
            return Ok(options);
        }
    }
    let input = sanitizer::options(scope, value, safe, true)?;
    if safe {
        return Ok(input);
    }
    let required = state
        .check_trusted_types_policy(&crate::trusted_types::PolicyCheck::SinkRequired)?
        == Some(true)
        || state.trusted_registry.borrow().requires(TrustedKind::Html);
    if !required {
        return Ok(input);
    }
    let callback = state
        .trusted_policies
        .borrow()
        .values()
        .find(|policy| policy.name == "default")
        .and_then(|policy| policy.create_parser_options.clone());
    if let Some(callback) = callback {
        let kind = v8_string(scope, "TrustedParserOptions")?;
        let sink = v8_string(scope, sink)?;
        if let Some(mut result) = call(scope, callback, &input, &[kind.into(), sink.into()])? {
            // Default policy may disable requested script execution, never enable it.
            result.run_scripts &= input.run_scripts;
            return Ok(result);
        }
        // Nullable default-policy results follow the same native violation /
        // report-only decision as a missing callback; thrown exceptions escape.
    }
    if state.check_trusted_types_policy(&crate::trusted_types::PolicyCheck::Sink {
        sink: sink.into(),
        input: String::new(),
        location: trusted_source_location(scope),
    })? == Some(true)
    {
        return Ok(input);
    }
    Err(invalid())
}
pub(super) fn get_run_scripts(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    match record(scope, args.this().into()) {
        Some(options) => rv.set_bool(options.run_scripts),
        None => throw_type_error(scope, "incompatible TrustedParserOptions receiver"),
    }
}
pub(super) fn create(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let result = (|| -> Result<_, V8BindingError> {
        let (state, id) = trusted_policy_owner(scope, args.this()).ok_or_else(invalid)?;
        let input = sanitizer::options(scope, args.get(0), false, true)?;
        let callback = state
            .trusted_policies
            .borrow()
            .get(&id)
            .and_then(|p| p.create_parser_options.clone())
            .ok_or_else(invalid)?;
        let extra = (1..args.length())
            .map(|index| v8::Local::new(scope, args.get(index)))
            .collect::<Vec<_>>();
        let options = call(scope, callback, &input, &extra)?.ok_or_else(invalid)?;
        if state.execution_retired() {
            return Err(invalid());
        }
        let context = args
            .this()
            .get_creation_context(scope)
            .ok_or_else(invalid)?;
        let scope = &mut v8::ContextScope::new(scope, context);
        let template = v8::ObjectTemplate::new(scope);
        template.set_internal_field_count(3);
        let object = template.new_instance(scope).ok_or_else(invalid)?;
        set_trusted_prototype(scope, &state, object, "TrustedParserOptions")?;
        let id = state.next_trusted_id.get();
        let next = id.checked_add(1).ok_or_else(invalid)?;
        let brand = v8_string(scope, "TrustedParserOptions")?;
        object.set_internal_field(0, brand.into());
        object.set_internal_field(1, v8::BigInt::new_from_u64(scope, id).into());
        object.set_internal_field(2, v8::BigInt::new_from_u64(scope, state.instance_id).into());
        let records = context.get_slot::<Records>().unwrap_or_else(|| {
            let records = Rc::new(Records::default());
            context.set_slot(records.clone());
            records
        });
        let mut map = records.0.borrow_mut();
        map.retain(|_, (object, _)| object.to_local(scope).is_some());
        map.insert(id, (v8::Weak::new(scope, object), options));
        state.next_trusted_id.set(next);
        Ok(v8::Global::new(scope, object))
    })();
    match result {
        Ok(object) => rv.set(v8::Local::new(scope, object).into()),
        Err(error) => throw_binding_error(scope, &error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn run(source: &str) {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(
            scope,
            context,
            Rc::new(RefCell::new(
                oryn_dom::html::parse("<body>").into_runtime().unwrap(),
            )),
            RealmId(0),
            1,
        )
        .unwrap();
        state.configure_trusted_types(TrustedTypesConfig {
            require_script: true,
            allow_any_policy: true,
            ..Default::default()
        });
        let source = v8::String::new(scope, source).unwrap();
        let promise = v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
            .try_cast::<v8::Promise>()
            .unwrap();
        for _ in 0..256 {
            scope.perform_microtask_checkpoint();
            if promise.state() != v8::PromiseState::Pending {
                break;
            }
        }
        assert_eq!(
            promise.state(),
            v8::PromiseState::Fulfilled,
            "{}",
            promise.result(scope).to_rust_string_lossy(scope)
        );
        assert!(promise.result(scope).is_true());
    }
    #[test]
    fn required_parser_authority_is_checked_before_replacement_and_not_on_chunks() {
        run(r#"(async()=>{
            const host=document.createElement('div'),target=document.createElement('i');target.textContent='old';host.append(target);document.body.append(host);
            for(const position of ['', 'Append','Prepend','Before','After','ReplaceWith']){
                let threw=false;try{target['stream'+position+'HTMLUnsafe']()}catch(e){threw=e instanceof TypeError}
                if(!threw||target.textContent!=='old'||target.parentNode!==host)throw Error('missing synchronous authority');
                const writer=target['stream'+position+'HTML']().getWriter();await writer.write('<b>safe</b>');await writer.close();
                if(!target.parentNode)host.append(target);target.textContent='old';
            }
            const policy=trustedTypes.createPolicy('default',{createHTML(){throw Error('must not process chunk')},createParserOptions:o=>o});
            const writer=target.streamAppendHTMLUnsafe().getWriter();await writer.write('<u>first');await writer.write(' second</u>');await writer.close();
            if(target.textContent!=='oldfirst second')throw Error('plain chunks');return true;
        })()"#);
    }
    #[test]
    fn default_policy_cannot_enable_scripts_and_can_override_sanitizer() {
        run(r#"(async()=>{
            let calls=0;trustedTypes.createPolicy('default',{createParserOptions(o,kind,sink){calls++;if(kind!=='TrustedParserOptions'||!sink.includes('stream'))throw Error('policy arguments');return {runScripts:true,sanitizer:{removeElements:['span']}}}});
            globalThis.streamAuthorityRuns=0;
            const target=document.createElement('div');document.body.append(target);
            const writer=target.streamHTMLUnsafe({runScripts:false,sanitizer:{removeElements:['div']}}).getWriter();
            await writer.write('<div>ok<span>bad</span><script>streamAuthorityRuns++</script></div>');await writer.close();
            if(calls!==1||streamAuthorityRuns!==0||target.querySelector('span')||!target.querySelector('div'))throw Error('default policy contract');return true;
        })()"#);
    }
    #[test]
    fn explicit_parser_options_brand_and_readonly_descriptor_preserve_authority() {
        run(r#"(async()=>{
            const policy=trustedTypes.createPolicy('explicit',{createParserOptions:o=>o});
            const trusted=policy.createParserOptions({runScripts:true,sanitizer:{removeElements:['span']}});
            if(!(trusted instanceof TrustedParserOptions)||!trusted.runScripts)throw Error('brand');
            const descriptor=Object.getOwnPropertyDescriptor(TrustedParserOptions.prototype,'runScripts');
            if(!descriptor.get||descriptor.set||Object.hasOwn(trusted,'runScripts'))throw Error('descriptor');
            for(const fake of [{},Object.create(TrustedParserOptions.prototype),new Proxy(trusted,{})]){
                let threw=false;try{descriptor.get.call(fake)}catch(e){threw=e instanceof TypeError}if(!threw)throw Error('forged getter');
                const target=document.createElement('div');threw=false;try{target.streamHTMLUnsafe(fake)}catch(e){threw=true}if(!threw)throw Error('forged authority');
            }
            trustedTypes.createPolicy('default',{createParserOptions(){throw Error('explicit authority bypasses default')}});
            globalThis.explicitRuns=0;const target=document.createElement('div');document.body.append(target);
            const writer=target.streamHTMLUnsafe(trusted).getWriter();await writer.write('<script>explicitRuns++</script><span>bad</span><b>ok</b>');await writer.close();
            if(explicitRuns!==1||target.querySelector('span')||!target.querySelector('b'))throw Error('explicit policy');return true;
        })()"#);
    }
    #[test]
    fn parser_policy_throw_null_and_reentrant_adoption_do_not_clear_target() {
        run(r#"(async()=>{
            let mode='throw',target;const foreign=document.implementation.createHTMLDocument('');
            trustedTypes.createPolicy('default',{createParserOptions(o){if(mode==='throw')throw new RangeError('marker');if(mode==='null')return null;foreign.adoptNode(target);return o}});
            for(mode of ['throw','null','adopt']){
                target=document.createElement('div');target.textContent='old';document.body.append(target);
                let threw=false;try{target.streamHTMLUnsafe()}catch(e){threw=true;if(mode==='throw'&&!(e instanceof RangeError))throw e}
                if(!threw||target.textContent!=='old')throw Error('policy failure changed target');
            }return true;
        })()"#);
    }
    #[test]
    fn default_parser_callback_preserves_live_sanitizer_object_identity() {
        run(r#"(async()=>{
            const sanitizer=new Sanitizer({removeElements:['span']});
            trustedTypes.createPolicy('default',{createParserOptions(o){if(o.sanitizer!==sanitizer)throw Error('lost sanitizer identity');sanitizer.removeElement('b');return o}});
            const target=document.createElement('div'),writer=target.streamAppendHTMLUnsafe({sanitizer}).getWriter();
            await writer.write('<span>bad</span><b>bad</b><i>ok</i>');await writer.close();
            if(target.querySelector('span')||target.querySelector('b')||!target.querySelector('i'))throw Error('live config');return true;
        })()"#);
    }
}

#[cfg(test)]
mod native_policy_tests {
    use super::*;
    #[test]
    fn nullable_default_parser_policy_reports_and_obeys_native_enforcement() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope, &mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(
            scope,
            context,
            Rc::new(RefCell::new(
                oryn_dom::html::parse("<body>").into_runtime().unwrap(),
            )),
            RealmId(0),
            1,
        )
        .unwrap();
        let enforce = Rc::new(Cell::new(false));
        let checks = Rc::new(RefCell::new(Vec::new()));
        let observed = checks.clone();
        let enforcing = enforce.clone();
        state.set_trusted_types_policy(Rc::new(move |_, check| {
            observed.borrow_mut().push(check.clone());
            Ok(match check {
                crate::trusted_types::PolicyCheck::SinkRequired
                | crate::trusted_types::PolicyCheck::Policy { .. } => true,
                crate::trusted_types::PolicyCheck::Sink { .. } => !enforcing.get(),
            })
        }));
        let eval = |scope: &mut v8::PinScope, source: &str| {
            let source = v8::String::new(scope, source).unwrap();
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        };
        assert!(eval(
            scope,
            "globalThis.parserMode='null';globalThis.target=document.createElement('div');target.textContent='old';trustedTypes.createPolicy('default',{createParserOptions(){if(parserMode==='throw')throw new RangeError('original');return parserMode==='null'?null:undefined}});true"
        ));
        for enforcing in [false, true] {
            enforce.set(enforcing);
            for mode in ["null", "undefined"] {
                checks.borrow_mut().clear();
                let source = format!(
                    "parserMode='{mode}';(()=>{{let error;try{{target.streamAppendHTMLUnsafe()}}catch(e){{error=e}}return {}&&target.textContent==='old'}})()",
                    if enforcing {
                        "error instanceof TypeError"
                    } else {
                        "error===undefined"
                    }
                );
                assert!(eval(scope, &source));
                assert_eq!(checks.borrow().iter().filter(|check| matches!(check, crate::trusted_types::PolicyCheck::Sink { sink, .. } if sink == "Element streamAppendHTMLUnsafe")).count(), 1);
            }
        }
        checks.borrow_mut().clear();
        assert!(eval(
            scope,
            "parserMode='throw';(()=>{try{target.streamAppendHTMLUnsafe()}catch(e){return e instanceof RangeError&&e.message==='original'}return false})()"
        ));
        assert!(
            !checks
                .borrow()
                .iter()
                .any(|check| matches!(check, crate::trusted_types::PolicyCheck::Sink { .. }))
        );
        assert!(eval(
            scope,
            "parserMode='null';(()=>{try{trustedTypes.defaultPolicy.createParserOptions()}catch(e){return e instanceof TypeError}return false})()"
        ));
    }
}
