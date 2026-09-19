//! Web IDL conversion precedes Blob byte processing, including dictionary getters.
use super::*;
enum Part {
    Bytes(Vec<u8>),
    Buffer(v8::Global<v8::Value>),
    Text(String),
}
fn invalid() -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "Blob".into(),
        member: "TypeError".into(),
    }
}
fn string(scope: &mut v8::PinScope, value: v8::Local<v8::Value>) -> Result<String, V8BindingError> {
    Ok(value
        .to_string(scope)
        .ok_or(V8BindingError::JavaScriptException)?
        .to_rust_string_lossy(scope))
}
fn parts(
    scope: &mut v8::PinScope,
    input: v8::Local<v8::Value>,
    optional: bool,
) -> Result<Vec<Part>, V8BindingError> {
    if optional && input.is_undefined() {
        return Ok(Vec::new());
    }
    let object = v8::Local::<v8::Object>::try_from(input).map_err(|_| invalid())?;
    let key = v8::Symbol::get_iterator(scope);
    let method = object
        .get(scope, key.into())
        .ok_or(V8BindingError::JavaScriptException)?;
    let method = v8::Local::<v8::Function>::try_from(method).map_err(|_| invalid())?;
    let mut parts = Vec::new();
    iterable_conversion::consume(scope, object, method, |scope, value| {
        if value.is_shared_array_buffer() {
            return Err(invalid());
        }
        if value.is_array_buffer() || value.is_array_buffer_view() {
            let buffer = if let Ok(view) = v8::Local::<v8::ArrayBufferView>::try_from(value) {
                view.buffer(scope).ok_or_else(invalid)?
            } else {
                v8::Local::<v8::ArrayBuffer>::try_from(value).map_err(|_| invalid())?
            };
            let backing = buffer.get_backing_store();
            if backing.is_shared() || backing.is_resizable_by_user_javascript() {
                return Err(invalid());
            }
            parts.push(Part::Buffer(v8::Global::new(scope, value)));
            return Ok(());
        }
        if let Ok(object) = v8::Local::<v8::Object>::try_from(value) {
            if let Some(owner) = binding_state_for_receiver(scope, object) {
                if let Ok(id) = stateful_id(scope, object) {
                    if let Ok(record) = platform_object(&owner, id) {
                        if let PlatformObjectRecord::Blob(blob) = &*record {
                            parts.push(Part::Bytes(blob.bytes.clone()));
                            return Ok(());
                        }
                    }
                }
            }
        }
        parts.push(Part::Text(string(scope, value)?));
        Ok(())
    })?;
    Ok(parts)
}
fn member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: Option<v8::Local<'s, v8::Object>>,
    name: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let Some(options) = options else {
        return Ok(v8::undefined(scope).into());
    };
    let key =
        v8::String::new(scope, name).ok_or(V8BindingError::Allocation("Blob dictionary key"))?;
    options
        .get(scope, key.into())
        .ok_or(V8BindingError::JavaScriptException)
}
pub(super) fn construct<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    args: &v8::FunctionCallbackArguments<'s>,
    file: bool,
) -> Result<PlatformObjectRecord, V8BindingError> {
    if file && args.length() < 2 {
        return Err(invalid());
    }
    let parts = parts(scope, args.get(0), !file)?;
    let name = if file {
        let value = args
            .get(1)
            .to_string(scope)
            .ok_or(V8BindingError::JavaScriptException)?;
        Some(oryn_dom::dom_string::usv(&rust_string_preserving_utf16(
            scope, value,
        )?))
    } else {
        None
    };
    let input = args.get(if file { 2 } else { 1 });
    let options = if input.is_null_or_undefined() {
        None
    } else {
        Some(v8::Local::<v8::Object>::try_from(input).map_err(|_| invalid())?)
    };
    // Convert base dictionaries before derived dictionaries, with members in
    // lexical order within each. Byte processing follows all conversions.
    let ending = member(scope, options, "endings")?;
    let ending = if ending.is_undefined() {
        "transparent".into()
    } else {
        string(scope, ending)?
    };
    if !matches!(ending.as_str(), "transparent" | "native") {
        return Err(invalid());
    }
    let media_type = member(scope, options, "type")?;
    let media_type = if media_type.is_undefined() {
        String::new()
    } else {
        normalize_media_type(string(scope, media_type)?)
    };
    let last_modified = if file {
        let value = member(scope, options, "lastModified")?;
        if value.is_undefined() {
            file_timestamp()
        } else {
            let number = value
                .number_value(scope)
                .ok_or(V8BindingError::JavaScriptException)?;
            let unsigned = if number.is_finite() {
                number.trunc().rem_euclid(18446744073709551616.0)
            } else {
                0.0
            };
            if unsigned >= 9223372036854775808.0 {
                (unsigned - 18446744073709551616.0) as i64
            } else {
                unsigned as i64
            }
        }
    } else {
        0
    };
    let mut bytes = Vec::new();
    for part in parts {
        match part {
            Part::Bytes(value) => bytes.extend(value),
            Part::Buffer(value) => {
                let value = v8::Local::new(scope, value);
                bytes.extend(bytes_from_value(scope, state, value)?);
            }
            Part::Text(mut value) => {
                if ending == "native" {
                    value = value.replace("\r\n", "\n").replace('\r', "\n");
                    if cfg!(windows) {
                        value = value.replace('\n', "\r\n");
                    }
                }
                bytes.extend(value.as_bytes());
            }
        }
    }
    Ok(PlatformObjectRecord::Blob(BlobRecord {
        bytes,
        media_type,
        file: name.map(|name| FileRecord {
            name,
            last_modified,
        }),
    }))
}

fn clamped_index(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<i64, V8BindingError> {
    let value = value
        .number_value(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    Ok(if value.is_nan() {
        0
    } else {
        value
            .clamp(i64::MIN as f64, i64::MAX as f64)
            .round_ties_even() as i64
    })
}
pub(super) fn slice(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    id: u64,
    args: &v8::FunctionCallbackArguments,
) -> Result<u64, V8BindingError> {
    if !matches!(&*platform_object(state, id)?, PlatformObjectRecord::Blob(_)) {
        return Err(invalid());
    }
    // Numeric/string conversion can call author code. Keep no native table
    // borrow across it and preserve exceptions before accessing the bytes.
    let start = clamped_index(scope, args.get(0))?;
    let end = if args.get(1).is_undefined() {
        None
    } else {
        Some(clamped_index(scope, args.get(1))?)
    };
    let media_type = if args.get(2).is_undefined() {
        String::new()
    } else {
        normalize_media_type(string(scope, args.get(2))?)
    };
    let bytes = {
        let record = platform_object(state, id)?;
        let PlatformObjectRecord::Blob(blob) = &*record else {
            return Err(invalid());
        };
        let length = blob.bytes.len();
        let normalize = |value: i64| {
            if value < 0 {
                (length as i128 + i128::from(value)).max(0) as usize
            } else {
                (value as usize).min(length)
            }
        };
        let start = normalize(start);
        let end = end.map(normalize).unwrap_or(length).max(start);
        blob.bytes[start..end].to_vec()
    };
    Ok(register_platform_object(
        state,
        PlatformObjectRecord::Blob(BlobRecord {
            bytes,
            media_type,
            file: None,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        super::super::test_eval::eval(scope, source)
    }
    // Use the same authenticated descriptor-backed factory as production;
    // Blob rejects the allocated object rather than an allocation failure.
    #[cfg(unix)]
    struct BlobSharedBroker;
    #[cfg(unix)]
    impl crate::worker_wire::WorkerTransport for BlobSharedBroker {
        fn request(
            &self,
            request: crate::worker_wire::AgentServiceRequest,
        ) -> Result<crate::worker_wire::AgentServiceResponse, crate::worker_wire::WorkerError>
        {
            use crate::shared_memory::*;
            use crate::worker_wire::*;
            let AgentServiceRequest::SharedMemory {
                creator: Creator::Agent(AgentId(98)),
                request,
            } = request
            else {
                panic!("wrong Blob fixture authority")
            };
            let mut descriptor = None;
            let mut native_descriptor = None;
            match request {
                SharedMemoryRequest::Bind { parent: None }
                | SharedMemoryRequest::Release { .. } => {}
                SharedMemoryRequest::Allocate {
                    byte_length: 4,
                    maximum_length: None,
                    granularity,
                    guard_length: 0,
                    wasm: false,
                    memory64: false,
                } => {
                    assert!(granularity.is_power_of_two() && granularity >= 4);
                    let mut token = [0; 32];
                    getrandom::fill(&mut token).unwrap();
                    let name = token
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>();
                    let path = std::env::temp_dir().join(format!("oryn-blob-shared-{name}"));
                    let file = std::fs::OpenOptions::new()
                        .read(true)
                        .write(true)
                        .create_new(true)
                        .open(&path)
                        .unwrap();
                    std::fs::remove_file(path).unwrap();
                    file.set_len(2 * granularity).unwrap();
                    // The production mapping ABI reads a native-endian atomic
                    // logical length in the metadata page before exposing bytes.
                    // A zero-filled file describes zero bytes, not this 4-byte SAB.
                    use std::os::unix::fs::FileExt;
                    file.write_all_at(&4u64.to_ne_bytes(), 0).unwrap();
                    let mut length = [0; 8];
                    file.read_exact_at(&mut length, 0).unwrap();
                    assert_eq!(u64::from_ne_bytes(length), 4);
                    native_descriptor = Some(std::sync::Arc::new(std::os::fd::OwnedFd::from(file)));
                    descriptor = Some(SharedMemoryDescriptor {
                        token: SharedMemoryToken(token),
                        byte_length: 4,
                        maximum_length: 4,
                        mapped_length: 2 * granularity,
                        data_offset: granularity,
                        reserved_length: granularity,
                        guard_length: 0,
                        wasm: false,
                        memory64: false,
                        growable: false,
                    });
                }
                other => panic!("unexpected Blob shared fixture request: {other:?}"),
            }
            Ok(AgentServiceResponse::SharedMemory(SharedMemoryResponse {
                isolated: true,
                outcome: SharedMemoryOutcome::None,
                descriptor,
                native_descriptor,
            }))
        }
    }
    #[test]
    fn blob_file_sequence_dictionary_order_and_foreign_parts() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let foreign = v8::Context::new(scope, Default::default());
        let (_owner, blob) = {
            let scope = &mut v8::ContextScope::new(scope, foreign);
            let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
            let state = install_generated_bindings(scope, foreign, dom, RealmId(97), 1).unwrap();
            let blob = eval(scope, "new Blob(['foreign'])");
            (state, v8::Global::new(scope, blob))
        };
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
        let state = install_generated_bindings(scope, context, dom, RealmId(98), 1).unwrap();
        let key = v8::String::new(scope, "foreignBlob").unwrap();
        let blob = v8::Local::new(scope, blob);
        context.global(scope).set(scope, key.into(), blob).unwrap();
        #[cfg(unix)]
        state.set_worker_transport(
            std::sync::Arc::new(BlobSharedBroker),
            Some(crate::worker_wire::Creator::Agent(
                crate::worker_wire::AgentId(98),
            )),
            None,
        );
        let value = eval(
            scope,
            r#"(async()=>{
            const check=(v,m)=>{if(!v)throw Error(m)};
            for(const parts of [null,true,1,'text',{}]){let failed=false;try{new Blob(parts)}catch(e){failed=e instanceof TypeError}check(failed,'sequence object required')}
            check(new Blob().size===0&&new Blob(undefined).size===0,'optional empty sequence');
            const original=new Blob(['abcd']);
            for(const [start,end,expected] of [[.5,undefined,'abcd'],[1.5,undefined,'cd'],[2.5,undefined,'cd'],[3.5,undefined,''],[0,1.5,'ab'],[0,2.5,'ab'],[-1.5,undefined,'cd'],[-Infinity,Infinity,'abcd'],[Infinity,undefined,''],[NaN,undefined,'abcd']])check(await original.slice(start,end).text()===expected,'clamped slice');
            const conversion=[];const sliceResult=original.slice({valueOf(){conversion.push('start');new Blob(['reentrant']);return 1.5}},
                {valueOf(){conversion.push('end');return 3.5}},{toString(){conversion.push('type');return 'TEXT/CUSTOM'}});
            check(conversion.join(',')==='start,end,type'&&sliceResult.type==='text/custom'&&await sliceResult.text()==='cd','slice reentrant conversions');
            const sliceError={};let sliceCaught,lateConversion=false;
            try{original.slice({valueOf(){throw sliceError}},{valueOf(){lateConversion=true;return 4}})}catch(e){sliceCaught=e}
            check(sliceCaught===sliceError&&!lateConversion&&original.size===4,'slice conversion exception');
            check(await new Blob(new Set([foreignBlob,null,undefined])).text()==='foreignnullundefined','iterable and native foreign Blob');
            let closed=0;try{new Blob({*[Symbol.iterator](){try{yield Symbol()}finally{closed++}}})}catch(e){check(e instanceof TypeError,'union string conversion')}check(closed===1,'iterator closed');
            const order=[],buffer=new Uint8Array([65]);
            const file=new File({*[Symbol.iterator](){order.push('iterate');yield buffer;yield {toString(){order.push('part');return '\r\nx'}}}},
                {toString(){order.push('name');return 'n\uD800'}},
                {get endings(){order.push('endings');return 'transparent'},get lastModified(){order.push('lastModified');return 12.9},get type(){order.push('type');buffer[0]=66;return 'TEXT/PLAIN'}});
            check(order.join(',')==='iterate,part,name,endings,type,lastModified','conversion order');
            check(file.name==='n\uFFFD'&&file.lastModified===12&&file.type==='text/plain','File metadata conversion');
            check(await file.text()==='B\r\nx','buffer processing after options and transparent endings');
            const marker={};let caught;try{new Blob([],{get endings(){throw marker},get type(){throw Error('late')}})}catch(e){caught=e}check(caught===marker,'original dictionary error');
            check(new Blob(['x'],{type:null}).type==='null','DOMString null');
            const binary=new Blob([new Uint8Array([0,255,65])]);const first=await binary.bytes(),second=await binary.bytes();
            check(first instanceof Uint8Array&&first!==second&&first.buffer!==second.buffer&&first.join(',')==='0,255,65','fresh byte results');first[0]=9;check(second[0]===0&&(await binary.bytes())[0]===0,'independent byte ownership');
            const stream=new Blob([new Uint8Array([239,187,191,65,255])]).textStream();const reader=stream.getReader();
            check((await reader.read()).value==='A\uFFFD'&&(await reader.read()).done,'decoded text stream');
            check((await new Blob().textStream().getReader().read()).done,'empty text stream');
            const detached=new ArrayBuffer(4),view=new Uint8Array(detached,1,2);detached.transfer();
            check(new Blob([detached,view,'tail']).size===4,'detached parts empty');
            const late=new ArrayBuffer(3);new Uint8Array(late).set([65,66,67]);
            check(new Blob([late],{get type(){late.transfer();return ''}}).size===0,'detach during dictionary conversion');
            const sliced=new Uint8Array([65,66,67,68]);check(await new Blob([sliced.subarray(1,3)]).text()==='BC','view window');
            const rab=new ArrayBuffer(4,{maxByteLength:8});
            for(const part of [rab,new Uint8Array(rab)]){let failed=false,closed=0;try{new Blob({*[Symbol.iterator](){try{yield part}finally{closed++}}})}catch(e){failed=e instanceof TypeError}check(failed&&closed===1,'resizable buffer rejection closes iterator')}
            {
                const shared=new SharedArrayBuffer(4);
                check(shared instanceof SharedArrayBuffer&&shared.byteLength===4,'genuine shared input');
                for(const part of [shared,new Uint8Array(shared)]){let failed=false;try{new Blob([part])}catch(e){failed=e instanceof TypeError}check(failed,'shared buffer rejected')}
            }
            return true;
        })()"#,
        );
        let promise = v8::Local::<v8::Promise>::try_from(value).unwrap();
        scope.perform_microtask_checkpoint();
        let result = promise.result(scope);
        let reason = result.to_rust_string_lossy(scope);
        let stack = if let Ok(object) = v8::Local::<v8::Object>::try_from(result) {
            let key = v8::String::new(scope, "stack").unwrap();
            object
                .get(scope, key.into())
                .map(|value| value.to_rust_string_lossy(scope))
        } else {
            None
        };
        assert_eq!(
            promise.state(),
            v8::PromiseState::Fulfilled,
            "{reason}; stack={stack:?}"
        );
        assert!(promise.result(scope).is_true());
        let blob = eval(
            scope,
            r"new Blob([new Uint8Array([13,10]),'\r\n'],{endings:'native'})",
        );
        let object = v8::Local::<v8::Object>::try_from(blob).unwrap();
        let id = stateful_id(scope, object).unwrap();
        let record = platform_object(&state, id).unwrap();
        let PlatformObjectRecord::Blob(blob) = &*record else {
            panic!("Blob")
        };
        assert_eq!(
            blob.bytes,
            if cfg!(windows) {
                b"\r\n\r\n".to_vec()
            } else {
                b"\r\n\n".to_vec()
            }
        );
    }
}
