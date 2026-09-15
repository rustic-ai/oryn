//! Native transform controller and readable-side pressure coupling.
use super::*;

pub(super) fn construct<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Result<(), V8BindingError> {
    let readable_hwm = high_water_mark(scope, args.get(2), 0.0)?;
    let writable_hwm = high_water_mark(scope, args.get(1), 1.0)?;
    let transformer = args.get(0);
    let (transform_id, _, writable_id) =
        create(scope, state, transformer, writable_hwm, readable_hwm)?;
    bind_platform_object(scope, state, transform_id, args.this());
    let start = option_function(scope, transformer, "start");
    writable_streams::start(scope, state, writable_id, start)?;
    Ok(())
}

pub(super) fn create<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    transformer: v8::Local<'s, v8::Value>,
    writable_hwm: f64,
    readable_hwm: f64,
) -> Result<(u64, u64, u64), V8BindingError> {
    let controller_id = reserve_platform_object_id(state);
    let transform_id = reserve_platform_object_id(state);
    let readable_id = reserve_platform_object_id(state);
    let readable_controller_id = reserve_platform_object_id(state);
    let writable_id = reserve_platform_object_id(state);
    let writable_controller_id = reserve_platform_object_id(state);
    insert_platform_object(
        state,
        readable_controller_id,
        PlatformObjectRecord::ReadableController {
            stream_id: readable_id,
        },
    );
    insert_platform_object(
        state,
        readable_id,
        PlatformObjectRecord::ReadableStream(ReadableStreamRecord {
            error: None,
            reader_id: None,
            transform_writable_id: Some(writable_id),
            high_water_mark: readable_hwm,
            started: true,
            pulling: false,
            pull_again: false,
            source: None,
            queue: VecDeque::new(),
            waiting: VecDeque::new(),
            closed: false,
            locked: false,
            pull: None,
            cancel: None,
            controller_id: readable_controller_id,
            pipes: Vec::new(),
            host_stream_id: None,
            host_read_pending: false,
        }),
    );
    insert_platform_object(
        state,
        writable_controller_id,
        PlatformObjectRecord::WritableController {
            stream_id: writable_id,
        },
    );
    insert_platform_object(
        state,
        writable_id,
        PlatformObjectRecord::WritableStream(WritableStreamRecord {
            error: None,
            queue: VecDeque::new(),
            busy: false,
            closing: false,
            started: false,
            controller_id: writable_controller_id,
            source: v8::Global::new(scope, transformer),
            pending_abort: None,
            abort_in_flight: false,
            abort_completion: None,
            writer_id: None,
            high_water_mark: writable_hwm,
            locked: false,
            closed: false,
            kind: WritableKind::Transform {
                codec_id: None,
                controller_id,
                transform: option_function(scope, transformer, "transform"),
                flush: option_function(scope, transformer, "flush"),
                readable_controller_id,
            },
        }),
    );
    insert_platform_object(
        state,
        transform_id,
        PlatformObjectRecord::TransformStream {
            readable_id,
            writable_id,
        },
    );
    insert_platform_object(
        state,
        controller_id,
        PlatformObjectRecord::TransformController {
            readable_id,
            readable_controller_id,
            writable_id,
        },
    );
    Ok((transform_id, readable_id, writable_id))
}

pub(super) fn high_water_mark<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    strategy: v8::Local<'s, v8::Value>,
    default: f64,
) -> Result<f64, V8BindingError> {
    let strategy = if strategy.is_null_or_undefined() {
        None
    } else {
        Some(v8::Local::<v8::Object>::try_from(strategy).map_err(|_| {
            V8BindingError::EventCallback("Queuing strategy must be an object".into())
        })?)
    };
    let value = event_init_member(scope, strategy, "highWaterMark")
        .ok_or(V8BindingError::JavaScriptException)?;
    if value.is_undefined() {
        return Ok(default);
    }
    let value = value
        .number_value(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    if value.is_nan() || value < 0.0 {
        let message = v8::String::new(scope, "Invalid highWaterMark")
            .ok_or(V8BindingError::Allocation("queue error"))?;
        let error = v8::Exception::range_error(scope, message);
        scope.throw_exception(error);
        return Err(V8BindingError::JavaScriptException);
    }
    Ok(value)
}

pub(super) fn write_blocked(state: &V8BindingState, writable: u64) -> Result<bool, V8BindingError> {
    let controller = {
        let record = platform_object(state, writable)?;
        let PlatformObjectRecord::WritableStream(stream) = &*record else {
            return Err(V8BindingError::Allocation("WritableStream record"));
        };
        if stream.queue.front().is_none_or(|r| r.chunk.is_none()) {
            return Ok(false);
        }
        match &stream.kind {
            WritableKind::Transform {
                readable_controller_id,
                ..
            } => *readable_controller_id,
            _ => return Ok(false),
        }
    };
    let readable = match &*platform_object(state, controller)? {
        PlatformObjectRecord::ReadableController { stream_id } => *stream_id,
        _ => return Err(V8BindingError::Allocation("ReadableStream controller")),
    };
    backpressured(state, readable)
}

pub(super) fn backpressured(state: &V8BindingState, readable: u64) -> Result<bool, V8BindingError> {
    let record = platform_object(state, readable)?;
    let PlatformObjectRecord::ReadableStream(stream) = &*record else {
        return Err(V8BindingError::Allocation("ReadableStream record"));
    };
    Ok(stream.waiting.is_empty()
        && stream.pipes.is_empty()
        && stream.queue.len() as f64 >= stream.high_water_mark)
}

pub(super) fn desired_size<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &V8BindingState,
    id: u64,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let readable_id = match &*platform_object(state, id)? {
        PlatformObjectRecord::TransformController { readable_id, .. } => *readable_id,
        _ => return Err(V8BindingError::Allocation("TransformStream controller")),
    };
    let record = platform_object(state, readable_id)?;
    let PlatformObjectRecord::ReadableStream(stream) = &*record else {
        return Err(V8BindingError::Allocation("ReadableStream record"));
    };
    if stream.error.is_some() {
        return Ok(v8::null(scope).into());
    }
    let size = if stream.closed && stream.queue.is_empty() {
        0.0
    } else {
        stream.high_water_mark - stream.queue.len() as f64
    };
    Ok(v8::Number::new(scope, size).into())
}

pub(super) fn operation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    id: u64,
    member: &str,
    chunk: v8::Local<'s, v8::Value>,
) -> Result<(), V8BindingError> {
    let (readable, controller, writable) = match &*platform_object(state, id)? {
        PlatformObjectRecord::TransformController {
            readable_id,
            readable_controller_id,
            writable_id,
        } => (*readable_id, *readable_controller_id, *writable_id),
        _ => return Err(V8BindingError::Allocation("TransformStream controller")),
    };
    match member {
        "enqueue" => readable_enqueue(scope, state, controller, chunk),
        "error" => {
            readable_streams::error(scope, state, readable, chunk)?;
            writable_streams::error(scope, state, writable, chunk)
        }
        "terminate" => {
            readable_close(scope, state, controller)?;
            let reason = readable_streams::type_error(scope, "TransformStream terminated")?;
            writable_streams::error(scope, state, writable, reason)
        }
        _ => Err(V8BindingError::Allocation(
            "TransformStream controller operation",
        )),
    }
}

pub(super) fn cancel_readable(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    readable: u64,
    reason: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let writable = match &*platform_object(state, readable)? {
        PlatformObjectRecord::ReadableStream(stream) => stream.transform_writable_id,
        _ => None,
    };
    if let Some(writable) = writable {
        writable_streams::error(scope, state, writable, reason)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::writable_streams::tests::run;

    #[test]
    fn default_pressure_blocks_each_write_until_readable_demand() {
        run(r#"(async()=>{
            const check=(x,m)=>{if(!x)throw Error(m)};const events=[];
            const t=new TransformStream(),w=t.writable.getWriter(),r=t.readable.getReader();
            const a=w.write(1),b=w.write(2);a.then(()=>events.push('write1'));b.then(()=>events.push('write2'));
            for(let i=0;i<8;i++)await Promise.resolve();check(events.length===0,'initial pressure');
            const read1=r.read();read1.then(()=>events.push('read1'));check((await read1).value===1,'first chunk');await a;
            for(let i=0;i<8;i++)await Promise.resolve();check(events.join()==='read1,write1','second blocked');
            const read2=r.read();read2.then(()=>events.push('read2'));check((await read2).value===2,'second chunk');await b;
            check(events.join()==='read1,write1,read2,write2','write follows read');await w.close();check((await r.read()).done,'closed');return 'ok';
        })()"#);
    }

    #[test]
    fn transform_controller_is_branded_and_pressure_reflects_queue() {
        run(r#"(async()=>{
            const check=(x,m)=>{if(!x)throw Error(m)};let c;
            const t=new TransformStream({start(x){c=x},transform(x,controller){check(c===controller,'controller identity');controller.enqueue(x)}},undefined,{highWaterMark:2});
            check(c instanceof TransformStreamDefaultController,'controller brand');check(!(c instanceof ReadableStreamDefaultController),'distinct interface');
            const w=t.writable.getWriter(),r=t.readable.getReader();await w.write(1);await w.write(2);check(c.desiredSize===0,'full queue');
            let third=false;const p=w.write(3).then(()=>third=true);for(let i=0;i<6;i++)await Promise.resolve();check(!third,'full queue pressure');
            check((await r.read()).value===1,'queue first');await p;check(c.desiredSize===0,'queue refilled');
            c.enqueue(4);check(c.desiredSize===-1,'negative desired size');await w.close();
            for(const x of [2,3,4])check((await r.read()).value===x,'queue order');check((await r.read()).done,'drained close');return 'ok';
        })()"#);
    }

    #[test]
    fn cancellation_error_and_termination_unblock_pending_writes() {
        run(r#"(async()=>{
            const check=(x,m)=>{if(!x)throw Error(m)};
            for(const action of ['cancel','error','terminate']){
                let c;const reason={failure:action};const t=new TransformStream({start(x){c=x}}),w=t.writable.getWriter(),r=t.readable.getReader();
                const write=w.write(1);write.catch(()=>{});await Promise.resolve();
                if(action==='cancel')await r.cancel(reason);else if(action==='error')c.error(reason);else c.terminate();
                for(const p of [write,w.closed]){let e;try{await p}catch(x){e=x}check(action==='terminate'?e instanceof TypeError:e===reason,'writable failure')}
                if(action==='error'){let e;try{await r.read()}catch(x){e=x}check(e===reason,'readable error');check(c.desiredSize===null,'errored desired size')}
                else check((await r.read()).done,'readable closed');
                let thrown;try{c.enqueue(2)}catch(e){thrown=e}check(thrown instanceof TypeError,'no enqueue after terminal');
            }return 'ok';
        })()"#);
    }

    #[test]
    fn high_water_mark_validation_and_zero_writable_pressure() {
        run(r#"(async()=>{
            const check=(x,m)=>{if(!x)throw Error(m)};
            for(const value of [-1,NaN])for(const make of [()=>new TransformStream({}, {}, {highWaterMark:value}),()=>new ReadableStream({}, {highWaterMark:value}),()=>new WritableStream({}, {highWaterMark:value})]){
                let e;try{make()}catch(x){e=x}check(e instanceof RangeError,'invalid hwm');
            }
            const w=new WritableStream({}, {highWaterMark:0}).getWriter();check(w.desiredSize===0,'zero writable hwm');let ready=false;w.ready.then(()=>ready=true);
            await Promise.resolve();check(!ready,'zero hwm ready pending');await w.close();await w.ready;check(ready,'close relieves pressure');return 'ok';
        })()"#);
    }
}
