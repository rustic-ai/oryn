//! Custody for the native Fetch Body read-all-bytes operation. Its acquired lock
//! remains held: completing read-all-bytes does not release an author reader.
use super::stream_graph::Handle;
use super::*;

#[derive(Clone)]
pub(super) struct Source {
    owner: std::rc::Weak<V8BindingState>,
    pub(super) wrapper: stream_graph::Owned<v8::Object>,
    id: u64,
}
impl Source {
    pub(super) fn capture(
        scope: &mut v8::PinScope,
        owner: &Rc<V8BindingState>,
        id: u64,
    ) -> Result<Self, V8BindingError> {
        let wrapper = new_platform_object(scope, owner, "ReadableStream", id)?;
        Ok(Self {
            owner: Rc::downgrade(owner),
            wrapper: stream_graph::Owned::new(scope, wrapper),
            id,
        })
    }
}

pub(super) fn register(result: &Rc<V8BindingState>, source: &Rc<V8BindingState>, id: u64) {
    let mut registry = result.body_consumers.borrow_mut();
    registry.retain(|(source,id)|source.upgrade().is_some_and(|source|matches!(source.platform_objects.borrow().get(id),Some(PlatformObjectRecord::ReadableStream(stream)) if stream.consumer.is_some())));
    registry.push((Rc::downgrade(source), id));
}
fn abandon(lease: &Source) {
    let Some(owner) = lease.owner.upgrade() else {
        return;
    };
    let host_id = owner
        .platform_objects
        .borrow()
        .get(&lease.id)
        .and_then(|record| match record {
            PlatformObjectRecord::ReadableStream(stream) => stream.host_stream_id,
            _ => None,
        });
    if let Some(id) = host_id
        && let Some(host) = owner.platform_host.borrow().as_ref()
    {
        host.abandon_http_stream(id);
    }
}
fn defer_cancel(lease: Source) {
    abandon(&lease);
    if let Some(owner) = lease
        .owner
        .upgrade()
        .filter(|owner| !owner.execution_retired())
    {
        owner
            .deferred_platform_tasks
            .borrow_mut()
            .push_back(DeferredPlatformTask::CancelBodyConsumer { source: lease });
    }
}
pub(super) fn reject_retired(
    scope: &mut v8::PinScope,
    resolver: stream_graph::Owned<v8::PromiseResolver>,
) {
    let Some(resolver) = resolver.get(scope) else {
        return;
    };
    v8::tc_scope!(let tc,scope);
    let reason = dom_exception_value(tc, "AbortError").unwrap_or_else(|_| v8::undefined(tc).into());
    if tc.has_caught() {
        tc.reset();
    }
    resolver.reject(tc, reason);
    if tc.has_caught() {
        tc.reset();
    }
}
pub(super) fn cancel_source(scope: &mut v8::PinScope, source: Source) {
    let Some(owner) = source
        .owner
        .upgrade()
        .filter(|owner| !owner.execution_retired())
    else {
        return;
    };
    let Some(wrapper) = source.wrapper.get(scope) else {
        return;
    };
    let Some(context) = wrapper.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    v8::tc_scope!(let tc,scope);
    let reason = dom_exception_value(tc, "AbortError").unwrap_or_else(|_| v8::undefined(tc).into());
    if tc.has_caught() {
        tc.reset();
    }
    let _ = service_worker_events::cancel_body(tc, &owner, source.id, reason);
    if tc.has_caught() {
        tc.reset();
    }
}
pub(super) fn retire(state: &V8BindingState) {
    // Result realm retirement stops only consumers that still name this exact
    // result owner. Numeric stream identity alone never authorizes cancellation.
    let links = std::mem::take(&mut *state.body_consumers.borrow_mut());
    for (source, id) in links {
        if let Some(source) = source.upgrade() {
            let consumer = {
                let mut records = source.platform_objects.borrow_mut();
                match records.get_mut(&id) {
                    Some(PlatformObjectRecord::ReadableStream(stream))
                        if stream.consumer.as_ref().is_some_and(|consumer| {
                            std::ptr::eq(consumer.result_owner.as_ptr(), state)
                        }) =>
                    {
                        stream.consumer.take()
                    }
                    _ => None,
                }
            };
            if let Some(consumer) = consumer {
                defer_cancel(consumer.source);
            }
        }
    }
    // A foreign surviving result realm must receive the rejected read promise
    // before this source's stream tracing anchors can be retired.
    let consumers = {
        let mut records = state.platform_objects.borrow_mut();
        records
            .values_mut()
            .filter_map(|record| match record {
                PlatformObjectRecord::ReadableStream(stream) => stream.consumer.take(),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    for consumer in consumers {
        if let Some(result) = consumer
            .result_owner
            .upgrade()
            .filter(|result| !std::ptr::eq(result.as_ref(), state) && !result.execution_retired())
        {
            result.deferred_platform_tasks.borrow_mut().push_back(
                DeferredPlatformTask::RejectBodyConsumer {
                    resolver: consumer.resolver,
                },
            );
        }
        abandon(&consumer.source);
    }
}
pub(super) fn pump(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    id: u64,
) -> Result<(), V8BindingError> {
    let custody = {
        let record = platform_object(state, id)?;
        let PlatformObjectRecord::ReadableStream(stream) = &*record else {
            return Err(V8BindingError::Allocation("body consumer stream"));
        };
        stream
            .consumer
            .as_ref()
            .map(|consumer| (consumer.resolver.clone(), consumer.source.clone()))
    };
    let Some((resolver, lease)) = custody else {
        return Ok(());
    };
    let Some(resolver) = resolver.get(scope) else {
        return Ok(());
    };
    let Some(_source_wrapper) = lease.wrapper.get(scope) else {
        return Ok(());
    };
    // Locals retain terminal custody even while inner code takes the consumer
    // out of the record and author callbacks retire or cancel the source.
    v8::tc_scope!(let tc,scope);
    let result = (|| {
        let context = state
            .global_object
            .to_local(tc)
            .and_then(|global| global.get_creation_context(tc))
            .ok_or(V8BindingError::Allocation("body source context"))?;
        let scope: &mut v8::PinScope = tc;
        let scope = &mut v8::ContextScope::new(scope, context);
        pump_response_body_inner(scope, state, id)?;
        if state.execution_retired() {
            return Err(V8BindingError::Allocation("body source retired"));
        }
        Ok(())
    })();
    if result.is_err() || tc.has_caught() {
        let exception = tc.exception();
        if tc.has_caught() {
            tc.reset();
        }
        let reason = exception.unwrap_or_else(|| {
            dom_exception_value(
                tc,
                if state.execution_retired() {
                    "AbortError"
                } else {
                    "TypeError"
                },
            )
            .unwrap_or_else(|_| v8::undefined(tc).into())
        });
        if tc.has_caught() {
            tc.reset();
        }
        resolver.reject(tc, reason);
        // Never restore a consumer that a nested close/error/cancel removed.
        if let Ok(mut record) = platform_object_mut(state, id)
            && let PlatformObjectRecord::ReadableStream(stream) = &mut *record
        {
            stream.consumer.take();
            stream.host_read_pending = false;
        }
        abandon(&lease);
        if !state.execution_retired() {
            if let Some(context) = _source_wrapper.get_creation_context(tc) {
                let source_scope: &mut v8::PinScope = tc;
                let source_scope = &mut v8::ContextScope::new(source_scope, context);
                let _ = service_worker_events::cancel_body(source_scope, state, id, reason);
            }
            if tc.has_caught() {
                tc.reset();
            }
        }
    }
    Ok(())
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
    fn successful_full_read_retains_lock_and_does_not_cancel() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
        let _state = install_generated_bindings(scope, context, dom, RealmId(92), 1).unwrap();
        eval(
            scope,
            "globalThis.cancelled=0;globalThis.s=new ReadableStream({start(c){c.enqueue(new Uint8Array([65,66]));c.close()},cancel(){cancelled++}});globalThis.r=new Response(s);globalThis.p=r.text()",
        );
        let promise = v8::Local::<v8::Promise>::try_from(eval(scope, "p")).unwrap();
        assert_eq!(promise.state(), v8::PromiseState::Fulfilled);
        assert_eq!(promise.result(scope).to_rust_string_lossy(scope), "AB");
        assert!(eval(scope, "s.locked && r.bodyUsed && cancelled===0").boolean_value(scope));
    }
    #[test]
    fn native_consumer_failure_keeps_returned_promise_and_acquired_lock() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
        let state = install_generated_bindings(scope, context, dom, RealmId(88), 1).unwrap();
        let response = eval(scope, "new Response(new Uint8Array([1,2]))");
        let response = v8::Local::<v8::Object>::try_from(response).unwrap();
        let id = stateful_id(scope, response).unwrap();
        let stream = match &*platform_object(&state, id).unwrap() {
            PlatformObjectRecord::Response(response) => response.body_stream_id.unwrap(),
            _ => panic!(),
        };
        // Inject failure at the fallible queue-drain boundary after the native
        // pump has taken the consumer and copied its chunks.
        if let PlatformObjectRecord::ReadableStream(record) =
            &mut *platform_object_mut(&state, stream).unwrap()
        {
            record.controller_id = u64::MAX;
        }
        let result =
            response_body_operation_bounded(scope, &state, id, "arrayBuffer", None).unwrap();
        let promise = v8::Local::<v8::Promise>::try_from(result).unwrap();
        assert_eq!(promise.state(), v8::PromiseState::Rejected);
        assert!(
            matches!(&*platform_object(&state,stream).unwrap(),PlatformObjectRecord::ReadableStream(stream) if stream.consumer.is_none()&&stream.disturbed&&stream.locked)
        );
    }
    #[test]
    fn invalid_chunk_rejects_without_author_cancel_and_read_lock_remains() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
        let state = install_generated_bindings(scope, context, dom, RealmId(89), 1).unwrap();
        eval(
            scope,
            "globalThis.cancelled=0;globalThis.s=new ReadableStream({start(c){c.enqueue('bad')},cancel(){cancelled++}});globalThis.r=new Response(s);globalThis.p=r.text()",
        );
        let promise = v8::Local::<v8::Promise>::try_from(eval(scope, "p")).unwrap();
        assert_eq!(promise.state(), v8::PromiseState::Rejected);
        assert!(eval(scope, "cancelled===0 && s.locked && r.bodyUsed").boolean_value(scope));
        let response = v8::Local::<v8::Object>::try_from(eval(scope, "r")).unwrap();
        let id = stateful_id(scope, response).unwrap();
        let next = response_body_operation(scope, &state, id, "text").unwrap();
        assert_eq!(
            v8::Local::<v8::Promise>::try_from(next).unwrap().state(),
            v8::PromiseState::Rejected
        );
    }
    #[test]
    fn actual_source_realm_retirement_settles_surviving_body_promise() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let (source, stream) = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
            let source = install_generated_bindings(scope, first, dom, RealmId(94), 1).unwrap();
            let stream = eval(scope, "new ReadableStream({pull(){}})");
            (source, v8::Global::new(scope, stream))
        };
        let second = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, second);
        let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
        let result = install_generated_bindings(scope, second, dom, RealmId(95), 1).unwrap();
        let key = v8::String::new(scope, "foreignStream").unwrap();
        let stream = v8::Local::new(scope, &stream);
        second.global(scope).set(scope, key.into(), stream).unwrap();
        let value = eval(
            scope,
            "globalThis.bodyPromise=new Response(foreignStream).text();bodyPromise.catch(()=>{});bodyPromise",
        );
        let promise = v8::Local::<v8::Promise>::try_from(value).unwrap();
        assert_eq!(promise.state(), v8::PromiseState::Pending);
        source.retire_execution();
        assert!(source.execution_retired());
        result.drain_platform_completions(scope, 32).unwrap();
        scope.perform_microtask_checkpoint();
        assert_eq!(scope.get_current_context(), second);
        assert_eq!(promise.state(), v8::PromiseState::Rejected);
        let rejection = promise.result(scope).to_object(scope).unwrap();
        let key = v8::String::new(scope, "name").unwrap();
        assert_eq!(
            rejection
                .get(scope, key.into())
                .unwrap()
                .to_rust_string_lossy(scope),
            "AbortError"
        );
        assert!(!result.execution_retired());
    }
    #[test]
    fn source_retirement_rejects_surviving_foreign_result_and_result_retirement_cancels_source() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let (source, stream) = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
            let state = install_generated_bindings(scope, first, dom, RealmId(90), 1).unwrap();
            let stream = eval(scope, "new ReadableStream({pull(){}})");
            (state, v8::Global::new(scope, stream))
        };
        let second = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, second);
        let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
        let result = install_generated_bindings(scope, second, dom, RealmId(91), 1).unwrap();
        let key = v8::String::new(scope, "foreignStream").unwrap();
        let stream = v8::Local::new(scope, &stream);
        second.global(scope).set(scope, key.into(), stream);
        let object =
            v8::Local::<v8::Object>::try_from(eval(scope, "new Response(foreignStream)")).unwrap();
        let id = stateful_id(scope, object).unwrap();
        let promise = response_body_operation(scope, &result, id, "text").unwrap();
        let promise = v8::Local::<v8::Promise>::try_from(promise).unwrap();
        assert_eq!(promise.state(), v8::PromiseState::Pending);
        retire(&source);
        let task = result
            .deferred_platform_tasks
            .borrow_mut()
            .pop_front()
            .unwrap();
        let DeferredPlatformTask::RejectBodyConsumer { resolver } = task else {
            panic!("missing exact result rejection")
        };
        reject_retired(scope, resolver);
        assert_eq!(promise.state(), v8::PromiseState::Rejected);
        // Reuse the still-live test source for a distinct consumer. The helper
        // retirement above is isolated from broader realm teardown deliberately.
        let new_stream = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let stream = eval(scope, "new ReadableStream({pull(){}})");
            v8::Global::new(scope, stream)
        };
        let key = v8::String::new(scope, "foreignStream").unwrap();
        let new_stream = v8::Local::new(scope, &new_stream);
        second.global(scope).set(scope, key.into(), new_stream);
        let object =
            v8::Local::<v8::Object>::try_from(eval(scope, "new Response(foreignStream)")).unwrap();
        let id = stateful_id(scope, object).unwrap();
        response_body_operation(scope, &result, id, "text").unwrap();
        retire(&result);
        assert!(
            source
                .deferred_platform_tasks
                .borrow()
                .iter()
                .any(|task| matches!(task, DeferredPlatformTask::CancelBodyConsumer { .. }))
        );
    }
}
