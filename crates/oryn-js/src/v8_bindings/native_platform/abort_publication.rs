//! Abort marks the entire dependent set atomically, then retains each algorithm
//! and event continuation until publication completes in the owning realm.
use super::*;
use std::collections::VecDeque;
pub(super) struct Batch {
    targets: VecDeque<Target>,
    reason: v8::Global<v8::Value>,
}
struct Target {
    owner: std::rc::Weak<V8BindingState>,
    generation: u64,
    epoch: u64,
    id: u64,
    wrapper: v8::Global<v8::Object>,
    event: publication::Notification,
    stage: u8,
    algorithms: Option<v8::Global<v8::Array>>,
    index: u32,
}
impl Drop for Target {
    fn drop(&mut self) {
        if let Some(owner) = self.owner.upgrade() {
            self.event.cancel(&owner);
        }
    }
}
pub(in super::super) fn start(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    id: u64,
    reason: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let root = scope
        .get_slot::<BindingStateRegistry>()
        .and_then(|registry| registry.instances.borrow().get(&state.instance_id).cloned())
        .and_then(|owner| owner.upgrade())
        .ok_or(V8BindingError::Allocation("AbortSignal owner"))?;
    if matches!(&*platform_object(state,id)?,PlatformObjectRecord::AbortSignal(signal) if signal.aborted)
    {
        return poll(scope, &root).map(|_| ());
    }
    let mut pending = VecDeque::from([(root.clone(), id)]);
    let mut visited = BTreeSet::new();
    let mut targets = VecDeque::new();
    let res = {
        let platform = root.native_platform.borrow();
        platform.aborts.len().saturating_add(platform.abort_depth) >= 128
    };
    if res {
        return Err(V8BindingError::Allocation(
            "AbortSignal publication capacity",
        ));
    }
    root.native_platform
        .borrow_mut()
        .aborts
        .try_reserve(1)
        .map_err(|_| V8BindingError::Allocation("AbortSignal publication admission"))?;
    while let Some((owner, id)) = pending.pop_front() {
        if !visited.insert((owner.instance_id, id)) {
            continue;
        }
        if visited.len() > 4096 {
            return Err(V8BindingError::Allocation("AbortSignal dependent capacity"));
        }
        let (followers, foreign) = {
            let record = platform_object(&owner, id)?;
            let PlatformObjectRecord::AbortSignal(signal) = &*record else {
                return Err(V8BindingError::Allocation("AbortSignal record"));
            };
            if signal.aborted {
                continue;
            }
            (signal.followers.clone(), signal.foreign_followers.clone())
        };
        for id in followers {
            pending.push_back((owner.clone(), id));
        }
        for follower in foreign {
            if let Some(owner) = follower.owner.upgrade().filter(|owner| {
                !owner.execution_retired()
                    && owner.platform_objects.borrow().contains_key(&follower.id)
            }) {
                pending.push_back((owner, follower.id));
            }
        }
        let context = owner
            .global_object
            .to_local(scope)
            .and_then(|global| global.get_creation_context(scope))
            .ok_or(V8BindingError::Allocation("AbortSignal realm"))?;
        let scope = &mut v8::ContextScope::new(scope, context);
        let wrapper = new_platform_object(scope, &owner, "AbortSignal", id)?;
        let event = create_event_object(scope, &owner, "abort", false, false)?;
        let event = publication::prepare_platform_dispatch(scope, &owner, id, "abort", event)?;
        targets.push_back(Target {
            owner: Rc::downgrade(&owner),
            generation: owner.document_generation(),
            epoch: owner.document_authority_epoch(),
            id,
            wrapper: v8::Global::new(scope, wrapper),
            event,
            stage: 0,
            algorithms: None,
            index: 0,
        });
    }
    // Everything fallible precedes marking. Strong wrapper handles retain all
    // signals and the reason until their complete algorithm lists are drained.
    for target in &targets {
        if let Some(owner) = target.owner.upgrade()
            && let Some(PlatformObjectRecord::AbortSignal(signal)) =
                owner.platform_objects.borrow_mut().get_mut(&target.id)
        {
            signal.aborted = true;
            signal.reason = Some(platform_listener_graph::Value::new(scope, reason));
            signal.foreign_sources.clear();
            signal.foreign_followers.clear();
        }
    }
    if !targets.is_empty() {
        let mut platform = root.native_platform.borrow_mut();
        platform.aborts.push_back(Batch {
            targets,
            reason: v8::Global::new(scope, reason),
        });
        platform.abort_retry = None;
    }
    poll(scope, &root).map(|_| ())
}
impl Target {
    fn resume(
        &mut self,
        scope: &mut v8::PinScope,
        reason: v8::Local<v8::Value>,
    ) -> Result<bool, V8BindingError> {
        let Some(owner) = self.owner.upgrade().filter(|owner| {
            !owner.execution_retired()
                && owner.document_generation() == self.generation
                && owner.document_authority_epoch() == self.epoch
        }) else {
            return Ok(true);
        };
        let context = owner
            .global_object
            .to_local(scope)
            .and_then(|global| global.get_creation_context(scope))
            .ok_or(V8BindingError::Allocation("AbortSignal realm"))?;
        let scope = &mut v8::ContextScope::new(scope, context);
        let wrapper = v8::Local::new(scope, &self.wrapper);
        if self.stage == 0 {
            platform_listener_graph::refresh_all(scope, &owner)?;
            node_event_listeners::abort(scope, &owner, self.id)?;
            self.stage = 1;
        }
        if self.stage == 1 {
            let key = v8_string(scope, "__oryn_observable_aborts")?;
            self.stage = 2;
            v8::tc_scope!(let tc,scope);
            let value = wrapper.get(tc, key);
            if let Some(value) =
                value.and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
            {
                self.algorithms = Some(v8::Global::new(tc, value));
            }
            let exception = tc.exception();
            let message = tc.message();
            if !tc.is_execution_terminating() {
                tc.reset();
            }
            if let Some(exception) = exception {
                let _ = report_exception(tc, exception, message);
            }
        }
        if self.stage == 2 {
            if let Some(algorithms) = &self.algorithms {
                let algorithms = v8::Local::new(scope, algorithms);
                while self.index < algorithms.length() {
                    if scope.is_execution_terminating() {
                        return Ok(false);
                    }
                    let index = self.index;
                    self.index += 1;
                    v8::tc_scope!(let tc,scope);
                    let callback = algorithms.get_index(tc, index);
                    let exception = tc.exception();
                    let message = tc.message();
                    if !tc.is_execution_terminating() {
                        tc.reset();
                    }
                    if let Some(exception) = exception {
                        let _ = report_exception(tc, exception, message);
                    }
                    if owner.execution_retired() {
                        return Ok(true);
                    }
                    if let Some(callback) = callback.filter(|value| value.is_function()) {
                        global_events::invoke(tc, callback, wrapper.into(), &[]);
                    }
                }
            }
            self.stage = 3;
        }
        if owner.execution_retired() {
            return Ok(true);
        }
        if scope.is_execution_terminating() {
            return Ok(false);
        }
        if self.stage == 3 {
            fetch_later::abort(&owner, self.id);
            fetch_body::abort(scope, &owner, self.id, reason)?;
            self.stage = 4;
        }
        if self.stage == 4 {
            let operations=owner.pending_platform_operations.borrow().iter().filter_map(|(id,pending)|matches!(pending,PendingPlatformOperation::FetchStart{signal_id,..} if *signal_id==self.id).then_some(*id)).collect::<Vec<_>>();
            for operation in operations {
                let pending = owner
                    .pending_platform_operations
                    .borrow_mut()
                    .remove(&operation);
                if let Some(PendingPlatformOperation::FetchStart { resolver, .. }) = pending {
                    v8::Local::new(scope, &resolver).reject(scope, reason);
                }
                if let Some(host) = owner.platform_host.borrow().as_ref() {
                    host.cancel(operation);
                }
                owner
                    .cancelled_platform_operations
                    .borrow_mut()
                    .insert(operation);
            }
            // Native stream cancellation is idempotent, independent of author
            // callbacks, and handed directly to the host before event delivery.
            let streams = {
                let objects = owner.platform_objects.borrow();
                objects
                    .values()
                    .filter_map(|record| match record {
                        PlatformObjectRecord::Response(response)
                            if response.signal_id == Some(self.id) =>
                        {
                            response
                                .body_stream_id
                                .and_then(|id| objects.get(&id))
                                .and_then(|record| match record {
                                    PlatformObjectRecord::ReadableStream(stream) => {
                                        stream.host_stream_id
                                    }
                                    _ => None,
                                })
                        }
                        _ => None,
                    })
                    .collect::<BTreeSet<_>>()
            };
            for stream in streams {
                if let Some(host) = owner.platform_host.borrow().as_ref() {
                    host.abandon_http_stream(stream);
                }
            }
            self.stage = 5;
        }
        if !self.event.resume(scope, &owner)? {
            return Ok(false);
        }
        self.event.finish(scope, &owner);
        Ok(true)
    }
}
pub(super) fn poll(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<usize, V8BindingError> {
    if state
        .native_platform
        .borrow()
        .abort_retry
        .is_some_and(|deadline| deadline > std::time::Instant::now())
    {
        return Ok(0);
    }
    let mut count = 0;
    loop {
        let batch = { state.native_platform.borrow_mut().aborts.pop_front() };
        let Some(mut batch) = batch else {
            state.native_platform.borrow_mut().abort_retry = None;
            return Ok(count);
        };
        let reason = v8::Local::new(scope, &batch.reason);
        while let Some(target) = batch.targets.front_mut() {
            state.native_platform.borrow_mut().abort_depth += 1;
            let result = target.resume(scope, reason);
            {
                let mut platform = state.native_platform.borrow_mut();
                platform.abort_depth = platform.abort_depth.saturating_sub(1);
            }
            match result {
                Ok(true) => {
                    batch.targets.pop_front();
                    count += 1;
                }
                result => {
                    let mut platform = state.native_platform.borrow_mut();
                    platform.aborts.push_front(batch);
                    platform.abort_retry =
                        Some(std::time::Instant::now() + std::time::Duration::from_millis(25));
                    return result.map(|_| count);
                }
            }
        }
    }
}

pub(in super::super) fn pending(state: &V8BindingState) -> bool {
    !state.native_platform.borrow().aborts.is_empty()
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
    fn dependent_abort_recovery_keeps_reason_event_identity_and_algorithm_order() {
        super::super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "globalThis.controller=new AbortController();globalThis.dependent=AbortSignal.any([controller.signal]);globalThis.calls=[];globalThis.event=null;controller.signal.__oryn_observable_aborts=[()=>calls.push('algorithm')];controller.signal.addEventListener('abort',e=>{event=e;calls.push('first');if(!dependent.aborted)throw Error('dependent set not marked')},{once:true});controller.signal.addEventListener('abort',e=>{if(e!==event)throw Error('event replay');calls.push('second')});dependent.addEventListener('abort',()=>calls.push('dependent'))",
        );
        let object = v8::Local::<v8::Object>::try_from(eval(scope, "controller.signal")).unwrap();
        let id = stateful_id(scope, object).unwrap();
        let reason = v8::String::new(scope, "original").unwrap();
        publication::fail_after_next_callback();
        assert!(start(scope, &state, id, reason.into()).is_err());
        assert!(pending(&state));
        assert!(state.native_platform.borrow().abort_retry.is_some());
        state.native_platform.borrow_mut().abort_retry =
            Some(std::time::Instant::now() + std::time::Duration::from_secs(60));
        assert_eq!(poll(scope, &state).unwrap(), 0);
        assert!(pending(&state));
        assert!(
            eval(
                scope,
                "calls.join(',')==='algorithm,first'&&dependent.reason==='original'"
            )
            .is_true()
        );
        state.native_platform.borrow_mut().abort_retry = None;
        poll(scope, &state).unwrap();
        assert!(
            eval(
                scope,
                "calls.join(',')==='algorithm,first,second,dependent'&&event.currentTarget===null"
            )
            .is_true()
        );
        let reason = v8::String::new(scope, "replacement").unwrap();
        start(scope, &state, id, reason.into()).unwrap();
        assert!(
            eval(
                scope,
                "controller.signal.reason==='original'&&calls.length===4"
            )
            .is_true()
        );
    }
    #[test]
    fn failure_before_abort_mark_leaves_all_signals_live() {
        super::super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        eval(
            scope,
            "globalThis.c=new AbortController();globalThis.d=AbortSignal.any([c.signal]);globalThis.calls=0;c.signal.onabort=()=>calls++",
        );
        let object = v8::Local::<v8::Object>::try_from(eval(scope, "c.signal")).unwrap();
        let id = stateful_id(scope, object).unwrap();
        publication::fail_before_next_prepare();
        let reason = v8::undefined(scope);
        assert!(start(scope, &state, id, reason.into()).is_err());
        assert!(eval(scope, "!c.signal.aborted&&!d.aborted&&calls===0").is_true());
        assert!(!pending(&state));
        start(scope, &state, id, reason.into()).unwrap();
        assert!(eval(scope, "c.signal.aborted&&d.aborted&&calls===1").is_true());
    }
}
