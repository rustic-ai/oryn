//! Synchronous FileAPI reads over immutable native Blob storage. Worker subtype
//! exposure is admitted by the native agent bootstrap, never by page properties.
use super::*;
fn invalid() -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "FileReaderSync".into(),
        member: "TypeError".into(),
    }
}
pub(super) fn operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    id: u64,
    member: &str,
    args: &v8::FunctionCallbackArguments,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    if state.execution_retired()
        || !matches!(
            &*platform_object(state, id)?,
            PlatformObjectRecord::FileReaderSync
        )
    {
        return Err(invalid());
    }
    let value = args.get(0);
    if !value.is_object() || value.is_proxy() {
        return Err(invalid());
    }
    let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| invalid())?;
    if object.internal_field_count() < 2
        || !object
            .get_internal_field(scope, 0)
            .and_then(|value| v8::Local::<v8::Value>::try_from(value).ok())
            .is_some_and(|value| value.is_number())
    {
        return Err(invalid());
    }
    let owner = binding_state_for_receiver(scope, object).ok_or_else(invalid)?;
    let blob_id = stateful_id(scope, object)?;
    if !owner
        .platform_wrappers
        .borrow()
        .get(&blob_id)
        .and_then(|wrapper| wrapper.to_local(scope))
        .is_some_and(|wrapper| wrapper.strict_equals(object.into()))
    {
        return Err(invalid());
    }
    if owner.execution_retired()
        || !matches!(
            &*platform_object(&owner, blob_id)?,
            PlatformObjectRecord::Blob(_)
        )
    {
        return Err(invalid());
    }
    // Blob conversion precedes the optional author callback. Do not hold a
    // registry borrow across conversion; retirement is rechecked afterwards.
    let label = if member == "readAsText" && !args.get(1).is_undefined() {
        Some(
            args.get(1)
                .to_string(scope)
                .ok_or(V8BindingError::JavaScriptException)?
                .to_rust_string_lossy(scope),
        )
    } else {
        None
    };
    if state.execution_retired() || owner.execution_retired() {
        return Err(invalid());
    }
    let record = platform_object(&owner, blob_id)?;
    let PlatformObjectRecord::Blob(blob) = &*record else {
        return Err(invalid());
    };
    match member {
        "readAsArrayBuffer" => Ok(array_buffer_from_bytes(scope, &blob.bytes).into()),
        "readAsBinaryString" => {
            let units: Vec<u16> = blob.bytes.iter().map(|byte| u16::from(*byte)).collect();
            v8::String::new_from_two_byte(scope, &units, v8::NewStringType::Normal)
                .map(Into::into)
                .ok_or(V8BindingError::Allocation("FileReaderSync binary string"))
        }
        "readAsDataURL" => v8_scalar_string(
            scope,
            &format!(
                "data:{};base64,{}",
                if blob.media_type.is_empty() {
                    "application/octet-stream"
                } else {
                    &blob.media_type
                },
                BASE64.encode(&blob.bytes)
            ),
        ),
        "readAsText" => v8_scalar_string(
            scope,
            &file_reader::decode(&blob.bytes, label.as_deref(), &blob.media_type),
        ),
        _ => Err(invalid()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn worker_subtype_exposure_and_native_sync_reads() {
        crate::v8_bindings::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        for (global, kind, visible) in [
            (BindingGlobal::Window, None, false),
            (BindingGlobal::Worker, None, false),
            (
                BindingGlobal::Worker,
                Some(crate::worker_wire::AgentKind::Dedicated),
                true,
            ),
            (
                BindingGlobal::Worker,
                Some(crate::worker_wire::AgentKind::Shared),
                true,
            ),
            (
                BindingGlobal::Worker,
                Some(crate::worker_wire::AgentKind::Service),
                false,
            ),
        ] {
            let context = v8::Context::new(scope, Default::default());
            if let Some(kind) = kind {
                context.set_slot(Rc::new(WorkerExposureKind(kind)));
            }
            let scope = &mut v8::ContextScope::new(scope, context);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("").into_runtime().unwrap(),
            ));
            let root = dom.borrow().document();
            install_generated_bindings_with_root_for_global(
                scope,
                context,
                dom,
                RealmId(0),
                1,
                root,
                global,
            )
            .unwrap();
            let script = if visible {
                r#"
              const check=(x,m)=>{if(!x)throw Error(m)};
              const r=new FileReaderSync(), b=new Blob([new Uint8Array([0,128,255])]);
              check(Object.prototype.toString.call(r)==='[object FileReaderSync]','brand');
              check(r.readAsBinaryString(b).charCodeAt(2)===255,'binary');
              const a=r.readAsArrayBuffer(b);check(a instanceof ArrayBuffer&&a.byteLength===3&&a!==r.readAsArrayBuffer(b),'fresh buffer');
              check(r.readAsText(new Blob(['σ']))==='σ','scalar');
              check(r.readAsText(new Blob([new Uint8Array([128])],{type:'text/plain;charset=windows-1252'}))==='€','MIME');
              check(r.readAsText(new Blob([new Uint8Array([239,187,191,65])]),'windows-1252')==='A','BOM');
              check(r.readAsDataURL(new Blob())==='data:application/octet-stream;base64,','empty URL');
              const d=Object.getOwnPropertyDescriptor(FileReaderSync.prototype,'readAsText');
              check(d.enumerable&&d.writable&&d.configurable&&d.value.length===1,'descriptor');
              for(const bad of [{},new Proxy(b,{})]){let threw=false;try{r.readAsText(bad)}catch(e){threw=e instanceof TypeError}check(threw,'Blob conversion')}
              let threw=false;try{r.readAsText.call({},b)}catch(e){threw=e instanceof TypeError}check(threw,'reader receiver');
              let calls=0;try{r.readAsText({}, {toString(){calls++;return 'utf-8'}})}catch(e){}check(calls===0,'conversion order');
            "#
            } else {
                "if('FileReaderSync' in globalThis)throw Error('incorrect exposure')"
            };
            let source = v8::String::new(scope, script).unwrap();
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap();
        }
    }
}

#[cfg(test)]
mod realm_tests {
    use super::*;
    struct RetireSource(Rc<V8BindingState>);
    fn retire_source(
        scope: &mut v8::PinScope,
        _args: v8::FunctionCallbackArguments,
        mut rv: v8::ReturnValue,
    ) {
        let source = scope
            .get_current_context()
            .get_slot::<RetireSource>()
            .unwrap();
        source.0.retire_execution();
        rv.set(v8::String::new(scope, "utf-8").unwrap().into());
    }
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, text: &str) -> v8::Local<'s, v8::Value> {
        let text = v8::String::new(scope, text).unwrap();
        v8::Script::compile(scope, text, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn foreign_blob_exact_owner_result_realm_and_conversion_retirement() {
        crate::v8_bindings::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let foreign = v8::Context::new(scope, Default::default());
        let (source, blob) = {
            let scope = &mut v8::ContextScope::new(scope, foreign);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("").into_runtime().unwrap(),
            ));
            let source = install_generated_bindings(scope, foreign, dom, RealmId(80), 1).unwrap();
            let blob = eval(
                scope,
                "new Blob([new Uint8Array([65,128])],{type:'text/plain;charset=windows-1252'})",
            );
            (source, v8::Global::new(scope, blob))
        };
        let context = v8::Context::new(scope, Default::default());
        context.set_slot(Rc::new(WorkerExposureKind(
            crate::worker_wire::AgentKind::Dedicated,
        )));
        context.set_slot(Rc::new(RetireSource(source)));
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("").into_runtime().unwrap(),
        ));
        let root = dom.borrow().document();
        let state = install_generated_bindings_with_root_for_global(
            scope,
            context,
            dom,
            RealmId(81),
            1,
            root,
            BindingGlobal::Worker,
        )
        .unwrap();
        let blob = v8::Local::new(scope, &blob);
        let key = v8::String::new(scope, "foreignBlob").unwrap();
        context.global(scope).set(scope, key.into(), blob);
        let function = v8::Function::new(scope, retire_source).unwrap();
        let key = v8::String::new(scope, "retireSource").unwrap();
        context
            .global(scope)
            .set(scope, key.into(), function.into());
        eval(
            scope,
            r#"
            const check=(x,m)=>{if(!x)throw Error(m)};
            const decoy=new Blob(['WRONG']),reader=new FileReaderSync();
            check(reader.readAsText(foreignBlob)==='A€','foreign original bytes');
            const buffer=reader.readAsArrayBuffer(foreignBlob);check(buffer instanceof ArrayBuffer&&new Uint8Array(buffer)[1]===128,'result realm and bytes');
            const sentinel={};try{reader.readAsText(foreignBlob,{toString(){throw sentinel}});throw Error('missing throw')}catch(e){check(e===sentinel,'conversion exception')}
            let rejected=false;try{reader.readAsText(foreignBlob,{toString:retireSource})}catch(e){rejected=e instanceof TypeError}check(rejected,'retired during conversion');
            rejected=false;try{reader.readAsArrayBuffer(foreignBlob)}catch(e){rejected=e instanceof TypeError}check(rejected,'retired before read');
            check(reader.readAsText(decoy)==='WRONG','local decoy survives');
            check(reader.readAsText(new Blob([new Uint8Array([128])],{type:'text/plain;charset=windows-1252'}),'invalid-label')==='€','invalid label MIME fallback');
        "#,
        );
        assert!(!state.execution_retired());
    }
}
