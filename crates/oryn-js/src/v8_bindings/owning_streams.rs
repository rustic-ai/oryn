//! Owning readable chunks remain native serialized resources until consumed.
//! Cancellation/queue error drops these resources without creating JS frames.
use super::*;
pub(super) fn enabled(state: &V8BindingState, stream: u64) -> bool {
    matches!(state.platform_objects.borrow().get(&stream),Some(PlatformObjectRecord::ReadableStream(record))if record.owning)
}
pub(super) fn materialize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    chunk: stream_graph::Owned<v8::Value>,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    if let Some(value) = chunk.take_serialized() {
        structured_clone::deserialize(scope, value).ok_or(V8BindingError::JavaScriptException)
    } else {
        Ok(stream_graph::local(scope, chunk))
    }
}
pub(super) fn enqueue(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    stream: u64,
    controller: u64,
    value: v8::Local<v8::Value>,
    options: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    if !matches!(state.platform_objects.borrow().get(&stream),Some(PlatformObjectRecord::ReadableStream(record))if !record.closed&&record.error.is_none())
    {
        throw_type_error(scope, "Cannot enqueue into a closed or errored stream");
        return Err(V8BindingError::JavaScriptException);
    }
    let transfers = structured_clone::transfer_list(scope, options)
        .ok_or(V8BindingError::JavaScriptException)?;
    let (serialized, error) = {
        v8::tc_scope!(let tc,scope);
        let serialized = structured_clone::serialize(tc, value, &transfers);
        let error = tc.exception().map(|error| v8::Global::new(tc, error));
        (serialized, error)
    };
    let Some(serialized) = serialized else {
        let error = if let Some(error) = error {
            v8::Local::new(scope, &error)
        } else {
            dom_exception_value(scope, "DataCloneError")?
        };
        readable_streams::error(scope, state, stream, error)?;
        scope.throw_exception(error);
        return Err(V8BindingError::JavaScriptException);
    };
    let chunk = stream_graph::Owned::serialized(scope, serialized);
    readable_enqueue_chunk(scope, state, controller, chunk)
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
    fn owning_chunks_transfer_frames_dispose_on_cancel_and_clone_each_tee_branch() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            r#"globalThis.frame=()=>new VideoFrame(new Uint8Array([1,2,3,4,5,6,7,8,1,2,1,2]),{format:'I420',timestamp:1234,codedWidth:4,codedHeight:2});
            globalThis.failure='';globalThis.results=[];const original=frame();original.authored=1;
            globalThis.cancelledStream=new ReadableStream({type:'owning',start(c){c.enqueue(original,{transfer:[original]})}});
            if(original.format!==null||original.authored!==1)throw Error('source transfer');cancelledStream.cancel();
            const queued=frame(),buffer=new ArrayBuffer(4);new Uint8Array(buffer)[0]=7;
            const stream=new ReadableStream({type:'owning',start(c){c.enqueue({frame:queued,buffer},{transfer:[queued,buffer]});c.close()}});
            if(queued.format!==null||buffer.byteLength!==0)throw Error('enqueue transfer');
            const [left,right]=stream.tee();
            Promise.all([left.getReader().read(),right.getReader().read()]).then(([a,b])=>{
                if(a.value.frame===b.value.frame||a.value.frame.format!=='I420'||b.value.frame.format!=='I420')throw Error('frame branches');
                a.value.frame.close();if(b.value.frame.format!=='I420')throw Error('shared frame');b.value.frame.close();
                new Uint8Array(a.value.buffer)[0]=9;if(new Uint8Array(b.value.buffer)[0]!==7)throw Error('shared buffer');results.push('tee');
            }).catch(e=>failure=String(e));
            const closed=frame();closed.close();globalThis.cloneError='';
            const bad=new ReadableStream({type:'owning',start(c){try{c.enqueue(closed,{transfer:[closed]})}catch(e){cloneError=e.name}}});bad.getReader().read().catch(e=>results.push(e.name));
        "#,
        );
        for _ in 0..12 {
            scope.perform_microtask_checkpoint();
        }
        assert!(eval(scope,"failure===''&&cloneError==='DataCloneError'&&results.includes('tee')&&results.includes('DataCloneError')").is_true());
        let stream = eval(scope, "cancelledStream");
        let stream = v8::Local::<v8::Object>::try_from(stream).unwrap();
        let id = stateful_id(scope, stream).unwrap();
        assert!(
            matches!(&*platform_object(&state,id).unwrap(),PlatformObjectRecord::ReadableStream(record)if record.queue.is_empty())
        );
    }
    #[test]
    fn owning_enqueue_clones_graphs_and_propagates_getter_errors_without_partial_transfer() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body>").into_runtime().unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            r#"globalThis.seen=false;const marker={},buffer=new ArrayBuffer(4);let threw=false;
            const stream=new ReadableStream({type:'owning',start(c){try{c.enqueue({get x(){throw marker}},{transfer:[buffer]})}catch(e){threw=e===marker}}});
            if(!threw||buffer.byteLength!==4)throw Error('getter atomicity');stream.getReader().read().catch(e=>seen=e===marker);
        "#,
        );
        scope.perform_microtask_checkpoint();
        assert!(eval(scope, "seen").is_true());
        state.retire_execution();
    }
}
