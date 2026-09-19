//! File reading tasks own a byte snapshot and the native reader wrapper. They
//! survive document.open, but are discarded with the owning Window's task queue.
use super::*;

pub(super) struct ReadTask {
    id: u64,
    generation: u64,
    stage: u8,
    mode: String,
    bytes: Vec<u8>,
    media_type: String,
    encoding: Option<String>,
    _reader: v8::Global<v8::Object>,
    delivery: Option<Delivery>,
}
fn invalid() -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "FileReader".into(),
        member: "TypeError".into(),
    }
}
fn active(state: &V8BindingState, id: u64, generation: u64) -> bool {
    !state.execution_retired()
        && matches!(platform_object(state,id),Ok(record) if matches!(&*record,PlatformObjectRecord::FileReader{generation:g,ready_state:1,..} if *g==generation))
}
// A native target-only event cursor: completed listeners are never re-invoked
// after an infrastructure failure, including once-listener root refresh.
pub(super) struct Delivery {
    event: v8::Global<v8::Object>,
    event_id: u64,
    name: &'static str,
    phase: u8,
    listeners: Option<Vec<PlatformEventListener>>,
    next: usize,
    propagation_stopped: bool,
    pending_once: Option<(PlatformEventListener, v8::Global<v8::Value>)>,
}
impl Delivery {
    pub(super) fn from_event(
        scope: &mut v8::PinScope,
        value: v8::Local<v8::Object>,
        name: &'static str,
    ) -> Result<Self, V8BindingError> {
        let id = event_id(scope, value)?;
        Ok(Self {
            event: v8::Global::new(scope, value),
            event_id: id,
            name,
            phase: 0,
            listeners: None,
            next: 0,
            propagation_stopped: false,
            pending_once: None,
        })
    }
    fn new(
        scope: &mut v8::PinScope,
        state: &V8BindingState,
        name: &'static str,
        loaded: usize,
        total: usize,
    ) -> Result<Self, V8BindingError> {
        fault("event")?;
        let value =
            create_event_object_with_interface(scope, state, "ProgressEvent", name, false, false)?;
        let id = event_id(scope, value)?;
        if let Some(record) = state.events.borrow_mut().get_mut(&id) {
            record.progress = Some((true, loaded as f64, total as f64));
            record.is_trusted = true;
        }
        Ok(Self {
            event: v8::Global::new(scope, value),
            event_id: id,
            name,
            phase: 0,
            listeners: None,
            next: 0,
            propagation_stopped: false,
            pending_once: None,
        })
    }
    pub(super) fn resume(
        &mut self,
        scope: &mut v8::PinScope,
        state: &V8BindingState,
        id: u64,
        reader: &v8::Global<v8::Object>,
    ) -> Result<(), V8BindingError> {
        let event = v8::Local::new(scope, &self.event);
        let receiver = v8::Local::new(scope, reader);
        let _guard = begin_event_dispatch(state, self.event_id)?;
        {
            let mut events = state.events.borrow_mut();
            let record = events
                .get_mut(&self.event_id)
                .ok_or(V8BindingError::Allocation("FileReader event"))?;
            record.platform_target = Some(id);
            record.platform_current_target = Some(id);
            record.platform_path = vec![id];
            record.phase = 2;
        }
        while self.phase < 2 {
            if self.listeners.is_none() {
                self.listeners = Some(
                    state
                        .platform_listeners
                        .borrow()
                        .get(&(id, self.name.into()))
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|listener| listener.capture == (self.phase == 0))
                        .collect(),
                );
                self.next = 0;
            }
            while self.next < self.listeners.as_ref().unwrap().len() || self.pending_once.is_some()
            {
                if state.execution_retired() || scope.is_execution_terminating() {
                    self.phase = 2;
                    return Ok(());
                }
                let (listener, keepalive) = if let Some((listener, keepalive)) = &self.pending_once
                {
                    fault("once_refresh")?;
                    platform_listener_graph::refresh(scope, state, id)?;
                    (listener.clone(), Some(v8::Local::new(scope, keepalive)))
                } else {
                    let listener = self.listeners.as_ref().unwrap()[self.next].clone();
                    let present = state
                        .platform_listeners
                        .borrow()
                        .get(&(id, self.name.into()))
                        .is_some_and(|listeners| {
                            listeners
                                .iter()
                                .any(|entry| entry.registration_id == listener.registration_id)
                        });
                    let aborted = listener
                        .signal
                        .as_ref()
                        .is_some_and(node_event_listeners::aborted);
                    if !present || aborted {
                        self.next += 1;
                        continue;
                    }
                    let keepalive = listener.callback.local(scope);
                    if listener.once {
                        if let Some(listeners) = state
                            .platform_listeners
                            .borrow_mut()
                            .get_mut(&(id, self.name.into()))
                        {
                            listeners
                                .retain(|entry| entry.registration_id != listener.registration_id);
                        }
                        if let Some(callback) = keepalive {
                            self.pending_once =
                                Some((listener.clone(), v8::Global::new(scope, callback)));
                        }
                        fault("once_refresh")?;
                        platform_listener_graph::refresh(scope, state, id)?;
                    }
                    (listener, keepalive)
                };
                fault("listener")?;
                // Advance only once every fallible admission completed. Author
                // callbacks and their exceptions are consumed exactly once.
                self.next += 1;
                self.pending_once = None;
                state
                    .events
                    .borrow_mut()
                    .get_mut(&self.event_id)
                    .unwrap()
                    .in_passive_listener = listener.passive;
                if listener.global_handler {
                    let handler = state
                        .platform_handlers
                        .borrow()
                        .get(&(id, self.name.into()))
                        .cloned();
                    if let Some(handler) = handler {
                        let callback = handler.value(scope);
                        global_events::invoke(scope, callback, receiver.into(), &[event.into()]);
                    }
                } else if let Some(callback) = keepalive {
                    global_events::invoke(scope, callback, receiver.into(), &[event.into()]);
                }
                let stopped = {
                    let mut events = state.events.borrow_mut();
                    let record = events.get_mut(&self.event_id).unwrap();
                    record.in_passive_listener = false;
                    self.propagation_stopped |= record.propagation_stopped;
                    record.immediate_stopped
                };
                if stopped {
                    self.phase = 2;
                    return Ok(());
                }
            }
            if self.propagation_stopped {
                self.phase = 2;
                return Ok(());
            }
            self.phase += 1;
            self.listeners = None;
        }
        Ok(())
    }
}
#[cfg(test)]
thread_local! {pub(super) static FAILURE:RefCell<Option<(&'static str,usize)>>=const{RefCell::new(None)};}
fn fault(_point: &'static str) -> Result<(), V8BindingError> {
    #[cfg(test)]
    {
        let fail = FAILURE.with(|slot| {
            let mut slot = slot.borrow_mut();
            if let Some((point, left)) = slot.as_mut() {
                if *point == _point {
                    if *left == 0 {
                        *slot = None;
                        return true;
                    }
                    *left -= 1;
                }
            }
            false
        });
        if fail {
            return Err(V8BindingError::Allocation(
                "FileReader injected publication failure",
            ));
        }
    }
    Ok(())
}
pub(super) fn operation(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    id: u64,
    member: &str,
    args: &v8::FunctionCallbackArguments,
) -> Result<(), V8BindingError> {
    if member == "abort" {
        let loading = matches!(
            &*platform_object(state, id)?,
            PlatformObjectRecord::FileReader { ready_state: 1, .. }
        );
        if !loading {
            if let PlatformObjectRecord::FileReader { result, .. } =
                &mut *platform_object_mut(state, id)?
            {
                *result = None;
            }
            return Ok(());
        }
        // Acquire retry custody before removing the old read's tasks.
        let reader = new_platform_object(scope, state, "FileReader", id)?;
        let generation = {
            let mut record = platform_object_mut(state, id)?;
            let PlatformObjectRecord::FileReader {
                result,
                ready_state,
                generation,
            } = &mut *record
            else {
                return Err(invalid());
            };
            let next = generation
                .checked_add(1)
                .ok_or(V8BindingError::Allocation("FileReader generation"))?;
            *generation = next;
            *result = None;
            *ready_state = 2;
            next
        };
        state
            .deferred_platform_tasks
            .borrow_mut()
            .retain(|task| !matches!(task,DeferredPlatformTask::FileReader(read) if read.id==id));
        return step(
            scope,
            state,
            ReadTask {
                id,
                generation,
                stage: 5,
                mode: String::new(),
                bytes: Vec::new(),
                media_type: String::new(),
                encoding: None,
                _reader: v8::Global::new(scope, reader),
                delivery: None,
            },
        );
    }
    let value = args.get(0);
    if !value.is_object() || value.is_proxy() {
        return Err(invalid());
    }
    let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| invalid())?;
    let owner = binding_state_for_receiver(scope, object).ok_or_else(invalid)?;
    let blob_id = stateful_id(scope, object)?;
    // Native type conversion precedes optional author string conversion. Re-read
    // the reader state afterwards: toString may start or abort another read.
    if !matches!(
        &*platform_object(&owner, blob_id)?,
        PlatformObjectRecord::Blob(_)
    ) {
        return Err(invalid());
    }
    let encoding = if member == "readAsText" && !args.get(1).is_undefined() {
        Some(
            args.get(1)
                .to_string(scope)
                .ok_or(V8BindingError::JavaScriptException)?
                .to_rust_string_lossy(scope),
        )
    } else {
        None
    };
    if matches!(
        &*platform_object(state, id)?,
        PlatformObjectRecord::FileReader { ready_state: 1, .. }
    ) {
        return Err(V8BindingError::Unsupported {
            interface: "FileReader".into(),
            member: "InvalidStateError".into(),
        });
    }
    if state.deferred_platform_tasks.borrow().len() >= 1024 {
        return Err(V8BindingError::Allocation("FileReader task capacity"));
    }
    let reader = new_platform_object(scope, state, "FileReader", id)?;
    let (bytes, media_type) = {
        let record = platform_object(&owner, blob_id)?;
        let PlatformObjectRecord::Blob(blob) = &*record else {
            return Err(invalid());
        };
        (blob.bytes.clone(), blob.media_type.clone())
    };
    let generation = {
        let mut record = platform_object_mut(state, id)?;
        let PlatformObjectRecord::FileReader {
            result,
            ready_state,
            generation,
        } = &mut *record
        else {
            return Err(invalid());
        };
        let next = generation
            .checked_add(1)
            .ok_or(V8BindingError::Allocation("FileReader generation"))?;
        *generation = next;
        *result = None;
        *ready_state = 1;
        next
    };
    state
        .deferred_platform_tasks
        .borrow_mut()
        .push_back(DeferredPlatformTask::FileReader(ReadTask {
            id,
            generation,
            stage: 0,
            mode: member.into(),
            bytes,
            media_type,
            encoding,
            _reader: v8::Global::new(scope, reader),
            delivery: None,
        }));
    Ok(())
}
pub(super) fn decode(bytes: &[u8], label: Option<&str>, media_type: &str) -> String {
    let mime = crate::mime_type::MimeType::parse(media_type);
    let encoding = label
        .and_then(|label| encoding_rs::Encoding::for_label(label.as_bytes()))
        .or_else(|| {
            mime.as_ref()
                .and_then(|mime| mime.parameter("charset"))
                .and_then(|label| encoding_rs::Encoding::for_label(label.as_bytes()))
        })
        .unwrap_or(encoding_rs::UTF_8);
    // Encoding decode performs BOM sniffing before the selected fallback.
    encoding.decode(bytes).0.into_owned()
}
fn current(state: &V8BindingState, task: &ReadTask) -> bool {
    !state.execution_retired()
        && matches!(platform_object(state,task.id),Ok(record) if matches!(&*record,PlatformObjectRecord::FileReader{generation,..} if *generation==task.generation))
}
pub(super) fn step(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    mut task: ReadTask,
) -> Result<(), V8BindingError> {
    if !current(state, &task) {
        return Ok(());
    }
    let result = advance(scope, state, &mut task);
    match result {
        Ok(true) => {}
        Ok(false) => {
            if current(state, &task) {
                state
                    .deferred_platform_tasks
                    .borrow_mut()
                    .push_back(DeferredPlatformTask::FileReader(task));
            }
        }
        Err(error) => {
            if current(state, &task) {
                state
                    .deferred_platform_tasks
                    .borrow_mut()
                    .push_front(DeferredPlatformTask::FileReader(task));
            }
            return Err(error);
        }
    }
    Ok(())
}
fn advance(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    task: &mut ReadTask,
) -> Result<bool, V8BindingError> {
    let total = task.bytes.len();
    loop {
        if !current(state, task) {
            return Ok(true);
        }
        // The file reading task source reports progress only when bytes were
        // read. Empty blobs still dispatch loadstart, load and loadend.
        if task.stage == 1 && total == 0 {
            task.stage = 2;
        }
        if task.stage == 2 {
            if !active(state, task.id, task.generation) {
                return Ok(true);
            }
            let value = match task.mode.as_str() {
                "readAsArrayBuffer" => Ok(array_buffer_from_bytes(scope, &task.bytes).into()),
                "readAsBinaryString" => {
                    let units: Vec<u16> = task.bytes.iter().map(|byte| u16::from(*byte)).collect();
                    v8::String::new_from_two_byte(scope, &units, v8::NewStringType::Normal)
                        .map(Into::into)
                        .ok_or(V8BindingError::Allocation("FileReader binary string"))
                }
                "readAsDataURL" => v8_scalar_string(
                    scope,
                    &format!(
                        "data:{};base64,{}",
                        if task.media_type.is_empty() {
                            "application/octet-stream"
                        } else {
                            &task.media_type
                        },
                        BASE64.encode(&task.bytes)
                    ),
                ),
                _ => v8_scalar_string(
                    scope,
                    &decode(&task.bytes, task.encoding.as_deref(), &task.media_type),
                ),
            }?;
            {
                let mut record = platform_object_mut(state, task.id)?;
                let PlatformObjectRecord::FileReader {
                    result,
                    ready_state,
                    ..
                } = &mut *record
                else {
                    return Err(invalid());
                };
                *result = Some(v8::Global::new(scope, value));
                *ready_state = 2;
            }
            task.stage = 3;
        }
        let (name, loaded) = match task.stage {
            0 => ("loadstart", 0),
            1 => ("progress", total),
            3 => ("load", total),
            4 | 6 => ("loadend", total),
            5 => ("abort", 0),
            _ => return Ok(true),
        };
        if task.delivery.is_none() {
            task.delivery = Some(Delivery::new(scope, state, name, loaded, total)?);
        }
        task.delivery
            .as_mut()
            .unwrap()
            .resume(scope, state, task.id, &task._reader)?;
        task.delivery = None;
        let previous = task.stage;
        task.stage += 1;
        if previous == 4 || previous == 6 {
            return Ok(true);
        }
        if previous < 2 {
            return Ok(false);
        }
        // load/abort and their loadend share a task. A reentrant replacement
        // invalidates the old continuation even when publication later fails.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn text_encoding_bom_label_and_mime_precedence() {
        assert_eq!(
            decode(&[0xef, 0xbb, 0xbf, b'x'], Some("windows-1252"), ""),
            "x"
        );
        assert_eq!(decode(&[0xfe, 0xff, 0, 65], Some("utf-8"), ""), "A");
        assert_eq!(
            decode(
                &[0x80],
                Some("bad-label"),
                "text/plain;charset=windows-1252"
            ),
            "€"
        );
        assert_eq!(
            decode(&[0x80], Some("windows-1252"), "text/plain;charset=utf-8"),
            "€"
        );
    }
}

#[cfg(test)]
mod native_tests {
    use super::*;
    #[test]
    fn asynchronous_results_abort_reentry_and_binary_bytes() {
        crate::v8_bindings::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<main></main>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source=v8::String::new(scope,r#"
          const check=(v,m)=>{if(!v)throw Error(m)};const events=[];const r=new FileReader();
          r.onabort=()=>events.push('abort');r.onloadend=()=>events.push('end');r.abort();
          check(r.readyState===0&&events.length===0,'empty abort');
          r.onloadstart=e=>{check(e instanceof ProgressEvent&&e.isTrusted&&!e.bubbles&&!e.cancelable,'progress brand');check(r.result===null,'loading result');events.push('start')};
          r.onprogress=()=>events.push('progress');r.onload=()=>events.push('load');
          r.readAsBinaryString(new Blob([new Uint8Array([0,128,255])]));
          check(r.readyState===1&&r.result===null&&events.length===0,'asynchronous admission');
          let invalid=false;try{r.readAsText(new Blob())}catch(e){invalid=e.name==='InvalidStateError'}check(invalid,'active read');
        "#).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        for _ in 0..4 {
            state.drain_platform_completions(scope, 64).unwrap();
            scope.perform_microtask_checkpoint();
        }
        let source = v8::String::new(
            scope,
            r#"
          check(events.join(',')==='start,progress,load,end','ordered tasks');
          check(r.result.length===3&&r.result.charCodeAt(2)===255&&r.readyState===2,'binary data');
          events.length=0;r.readAsText(new Blob(['old']));r.abort();
          check(events.join(',')==='abort,end'&&r.result===null,'synchronous abort');
          r.onload=()=>{events.push('load');if(r.result==='one')r.readAsText(new Blob(['two']))};
          r.readAsText(new Blob(['one']));
        "#,
        )
        .unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        for _ in 0..8 {
            state.drain_platform_completions(scope, 64).unwrap();
            scope.perform_microtask_checkpoint();
        }
        let source=v8::String::new(scope,"check(r.result==='two'&&events.filter(e=>e==='end').length===2,'restart suppresses old loadend')").unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
    }
}

#[cfg(test)]
mod realm_tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn empty_blob_skips_progress_for_every_read_mode() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(96), 1).unwrap();
        eval(
            scope,
            r#"
            globalThis.reads=[];
            for(const mode of ['readAsText','readAsDataURL','readAsArrayBuffer','readAsBinaryString']){
                for(const content of ['', 'x']){
                    const reader=new FileReader(),events=[];
                    for(const type of ['loadstart','progress','load','loadend','error','abort'])reader.addEventListener(type,event=>events.push(event.type));
                    reads.push({reader,events,mode,empty:content===''});reader[mode](new Blob([content]));
                }
            }
            if(reads.some(item=>item.events.length||item.reader.readyState!==1))throw Error('synchronous event');
        "#,
        );
        for _ in 0..4 {
            state.drain_platform_completions(scope, 64).unwrap();
            scope.perform_microtask_checkpoint();
        }
        assert!(eval(scope,r#"reads.every(({reader,events,empty,mode})=>{
            if(events.join(',')!==(empty?'loadstart,load,loadend':'loadstart,progress,load,loadend')||reader.readyState!==2)return false;
            if(!empty)return true;
            return mode==='readAsArrayBuffer'?reader.result.byteLength===0:
                mode==='readAsDataURL'?reader.result==='data:application/octet-stream;base64,':reader.result==='';
        })"#).is_true());
    }
    #[test]
    fn foreign_blob_snapshot_and_retired_reader_task_custody() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let foreign = v8::Context::new(scope, Default::default());
        let (source_state, blob) = {
            let scope = &mut v8::ContextScope::new(scope, foreign);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            let state = install_generated_bindings(scope, foreign, dom, RealmId(91), 1).unwrap();
            let blob = eval(
                scope,
                "new Blob([new Uint8Array([0xef,0xbb,0xbf,65])],{type:'text/plain;charset=windows-1252'})",
            );
            (state, v8::Global::new(scope, blob))
        };
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(92), 1).unwrap();
        let key = v8::String::new(scope, "foreignBlob").unwrap();
        let blob = v8::Local::new(scope, &blob);
        context.global(scope).set(scope, key.into(), blob).unwrap();
        eval(
            scope,
            "const collision=new Blob(['WRONG']);const reader=new FileReader();let calls=0;reader.onload=()=>calls++;reader.readAsText(foreignBlob)",
        );
        // Accepted immutable Blob bytes survive the source realm's retirement.
        source_state.retire_execution();
        for _ in 0..4 {
            state.drain_platform_completions(scope, 64).unwrap();
            scope.perform_microtask_checkpoint();
        }
        assert!(eval(scope, "reader.result==='A'&&calls===1").is_true());
        eval(scope, "reader.readAsText(collision)");
        state.retire_execution();
        assert!(
            !state
                .deferred_platform_tasks
                .borrow()
                .iter()
                .any(|task| matches!(task, DeferredPlatformTask::FileReader(_)))
        );
        assert!(eval(scope, "calls===1").is_true());
    }
}

#[cfg(test)]
mod publication_tests {
    use super::*;
    fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
        let source = v8::String::new(scope, source).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
    }
    #[test]
    fn retained_delivery_resumes_once_listener_and_terminal_event_without_duplicate_prefix() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(93), 1).unwrap();
        eval(
            scope,
            "const calls=[];const r=new FileReader();r.addEventListener('loadstart',()=>calls.push('prefix'));r.addEventListener('loadstart',()=>calls.push('once'),{once:true});r.onload=()=>calls.push('load');r.onloadend=()=>calls.push('end');r.readAsText(new Blob(['value']))",
        );
        FAILURE.with(|slot| *slot.borrow_mut() = Some(("once_refresh", 0)));
        assert!(state.drain_platform_completions(scope, 1).is_err());
        assert!(
            eval(
                scope,
                "calls.join(',')==='prefix'&&r.readyState===1&&r.result===null"
            )
            .is_true()
        );
        assert!(
            state
                .deferred_platform_tasks
                .borrow()
                .iter()
                .any(|task| matches!(task, DeferredPlatformTask::FileReader(_)))
        );
        state.drain_platform_completions(scope, 1).unwrap();
        assert!(eval(scope, "calls.join(',')==='prefix,once'").is_true());
        state.drain_platform_completions(scope, 1).unwrap();
        // The load event succeeds; constructing loadend fails. Retrying must
        // preserve result identity and must not invoke load a second time.
        FAILURE.with(|slot| *slot.borrow_mut() = Some(("event", 1)));
        assert!(state.drain_platform_completions(scope, 1).is_err());
        assert!(
            eval(
                scope,
                "calls.join(',')==='prefix,once,load'&&r.result==='value'&&r.readyState===2"
            )
            .is_true()
        );
        state.drain_platform_completions(scope, 1).unwrap();
        assert!(eval(scope, "calls.join(',')==='prefix,once,load,end'").is_true());
        eval(scope, "calls.length=0;r.readAsText(new Blob(['next']))");
        for _ in 0..4 {
            state.drain_platform_completions(scope, 64).unwrap();
        }
        assert!(eval(scope, "calls.join(',')==='prefix,load,end'").is_true());
    }
    #[test]
    fn aborted_publication_retry_is_retired_by_replacement_and_empty_abort_has_no_event() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(94), 1).unwrap();
        eval(
            scope,
            "const r=new FileReader(),events=[];r.onabort=()=>events.push('abort');r.onloadend=()=>events.push('end');r.onload=()=>events.push('load');r.readAsText(new Blob(['old']))",
        );
        FAILURE.with(|slot| *slot.borrow_mut() = Some(("event", 1)));
        eval(
            scope,
            "try{r.abort()}catch(error){};if(events.join(',')!=='abort'||r.result!==null)throw Error('abort prefix');r.readAsText(new Blob(['replacement']))",
        );
        for _ in 0..5 {
            state.drain_platform_completions(scope, 64).unwrap();
        }
        assert!(
            eval(
                scope,
                "events.join(',')==='abort,load,end'&&r.result==='replacement'"
            )
            .is_true()
        );
        eval(scope, "r.abort()");
        assert!(
            eval(
                scope,
                "events.join(',')==='abort,load,end'&&r.result===null"
            )
            .is_true()
        );
    }
}
