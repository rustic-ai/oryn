//! Native Cache objects. Identity and Request/Response records never come from
//! author properties; observable dictionary and iterable conversion does.
use super::*;
use crate::{
    cache_wire::*,
    service_worker_wire::{ServiceRequest, ServiceResponse},
    worker_wire::{AgentServiceRequest, AgentServiceResponse},
};
struct Object {
    cache: Option<u64>,
    value: v8::Weak<v8::Object>,
}
enum ItemState {
    Fetch(v8::Global<v8::Promise>),
    Body(
        CacheResponse,
        v8::Global<v8::Promise>,
        Option<BodyStreamLease>,
    ),
    Done(CacheResponse),
}
struct Item {
    request: CacheRequest,
    state: ItemState,
    require_ok: bool,
    signal: Option<u64>,
}
struct Pending {
    cache: u64,
    items: Vec<Item>,
    resolver: v8::Global<v8::PromiseResolver>,
}
#[derive(Default)]
pub(super) struct Bindings {
    retired: bool,
    in_flight: usize,
    reclaimed: Rc<RefCell<std::collections::BTreeSet<u64>>>,
    objects: Vec<Object>,
    storage: Option<v8::Weak<v8::Object>>,
    cache_prototype: Option<v8::Weak<v8::Object>>,
    pending: Vec<Pending>,
}
fn failure(name: &str) -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "Cache".into(),
        member: name.into(),
    }
}
fn transport(
    state: &V8BindingState,
    operation: CacheOperation,
) -> Result<CacheResult, V8BindingError> {
    if retired(state) {
        return Err(failure("InvalidStateError"));
    }
    workers::publish_secure_context(state)?;
    if state.execution_retired() {
        return Err(failure("InvalidStateError"));
    }
    let creator = workers::native_creator(state)?;
    let transport = state
        .worker_transport
        .borrow()
        .clone()
        .ok_or_else(|| failure("InvalidStateError"))?;
    match transport.request(AgentServiceRequest::Service(ServiceRequest::Cache {
        creator,
        operation,
    })) {
        Ok(AgentServiceResponse::Service(ServiceResponse::Cache(result))) => Ok(result),
        Err(error) => Err(failure(&error.name)),
        _ => Err(failure("InvalidStateError")),
    }
}
fn receiver(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    value: v8::Local<v8::Object>,
) -> Result<Option<u64>, V8BindingError> {
    if value.is_proxy() {
        return Err(failure("TypeError"));
    }
    state
        .cache_storage
        .borrow()
        .objects
        .iter()
        .find(|object| {
            object
                .value
                .to_local(scope)
                .is_some_and(|object| object.strict_equals(value.into()))
        })
        .map(|object| object.cache)
        .ok_or_else(|| failure("TypeError"))
}
fn units(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<Vec<u16>, V8BindingError> {
    let value = value
        .to_string(scope)
        .ok_or(V8BindingError::JavaScriptException)?;
    let mut units = vec![0; value.length()];
    value.write_v2(scope, 0, &mut units, v8::WriteFlags::default());
    Ok(units)
}
fn query(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
    storage: bool,
) -> Result<(CacheQuery, Option<Vec<u16>>), V8BindingError> {
    let mut query = CacheQuery::default();
    let mut cache_name = None;
    if value.is_null_or_undefined() {
        return Ok((query, cache_name));
    }
    let object = value.to_object(scope).ok_or_else(|| failure("TypeError"))?;
    for name in ["cacheName", "ignoreMethod", "ignoreSearch", "ignoreVary"] {
        if name == "cacheName" && !storage {
            continue;
        }
        let key = v8::String::new(scope, name).unwrap();
        let value = object
            .get(scope, key.into())
            .ok_or(V8BindingError::JavaScriptException)?;
        match name {
            "cacheName" => {
                if !value.is_undefined() {
                    cache_name = Some(units(scope, value)?);
                }
            }
            "ignoreMethod" => query.ignore_method = value.boolean_value(scope),
            "ignoreSearch" => query.ignore_search = value.boolean_value(scope),
            _ => query.ignore_vary = value.boolean_value(scope),
        }
    }
    Ok((query, cache_name))
}
fn request_snapshot(
    scope: &mut v8::PinScope,
    state: &V8BindingState,
    input: v8::Local<v8::Value>,
) -> Result<(CacheRequest, u64), V8BindingError> {
    let undefined = v8::undefined(scope);
    let id = create_request_record(scope, state, input, undefined.into())?;
    let record = platform_object(state, id)?;
    let PlatformObjectRecord::Request(request) = &*record else {
        return Err(failure("TypeError"));
    };
    let headers = platform_object(state, request.headers_id)?;
    let PlatformObjectRecord::Headers(headers) = &*headers else {
        return Err(failure("TypeError"));
    };
    Ok((
        CacheRequest {
            url: request.url.clone(),
            method: request.method.clone(),
            headers: headers.entries.clone(),
        },
        id,
    ))
}
fn body(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    value: v8::Local<v8::Value>,
    require_ok: bool,
) -> Result<ItemState, V8BindingError> {
    let object = v8::Local::<v8::Object>::try_from(value).map_err(|_| failure("TypeError"))?;
    if object.is_proxy() {
        return Err(failure("TypeError"));
    }
    let owner = binding_state_for_receiver(scope, object).ok_or_else(|| failure("TypeError"))?;
    let state = &owner;
    let id = stateful_id(scope, object)?;
    if !state
        .platform_wrappers
        .borrow()
        .get(&id)
        .and_then(|weak| weak.to_local(scope))
        .is_some_and(|object| object.strict_equals(value))
    {
        return Err(failure("TypeError"));
    }
    let (metadata, stream, foreign) = {
        let record = platform_object(state, id)?;
        let PlatformObjectRecord::Response(response) = &*record else {
            return Err(failure("TypeError"));
        };
        if response.status == 206
            || response.response_type == "error"
            || (require_ok && !(200..300).contains(&response.status))
        {
            return Err(failure("TypeError"));
        }
        let headers = platform_object(state, response.headers_id)?;
        let PlatformObjectRecord::Headers(headers) = &*headers else {
            return Err(failure("TypeError"));
        };
        if headers.entries.iter().any(|(name, value)| {
            name.eq_ignore_ascii_case("vary") && value.split(',').any(|name| name.trim() == "*")
        }) {
            return Err(failure("TypeError"));
        }
        (
            CacheResponse {
                timing: response.timing.clone(),
                internal_response: response.internal_response.clone(),
                status: response.status,
                status_text: response.status_text.clone(),
                headers: headers.entries.clone(),
                body: Vec::new(),
                has_body: response.body_stream_id.is_some(),
                redirected: response.redirected,
                url: response.url.clone(),
                response_type: response.response_type.into(),
            },
            response.body_stream_id,
            response.body_owner.clone(),
        )
    };
    let stream = stream
        .map(|id| BodyStreamLease::capture(scope, &body_owner(state, &foreign)?, id))
        .transpose()?;
    let promise = response_body_operation_internal(
        scope,
        state,
        id,
        "arrayBuffer",
        Some(32 * 1024 * 1024),
        true,
    )?;
    let promise = v8::Local::<v8::Promise>::try_from(promise).map_err(|_| failure("TypeError"))?;
    Ok(ItemState::Body(
        metadata,
        v8::Global::new(scope, promise),
        stream,
    ))
}
fn wrap<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    id: u64,
) -> Result<v8::Local<'s, v8::Object>, V8BindingError> {
    let value = v8::Object::new(scope);
    let prototype = state
        .cache_storage
        .borrow()
        .cache_prototype
        .clone()
        .ok_or_else(|| failure("InvalidStateError"))?;
    let prototype = prototype
        .to_local(scope)
        .ok_or_else(|| failure("InvalidStateError"))?;
    value.set_prototype(scope, prototype.into());
    let reclaimed = state.cache_storage.borrow().reclaimed.clone();
    let weak = v8::Weak::with_finalizer(
        scope,
        value,
        Box::new(move |_| {
            reclaimed.borrow_mut().insert(id);
        }),
    );
    state.cache_storage.borrow_mut().objects.push(Object {
        cache: Some(id),
        value: weak,
    });
    Ok(value)
}
fn response<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    response: CacheResponse,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    let kind = match response.response_type.as_str() {
        "basic" => "basic",
        "cors" => "cors",
        "opaque" => "opaque",
        "opaqueredirect" => "opaqueredirect",
        _ => "default",
    };
    let headers_id = register_platform_object(
        state,
        PlatformObjectRecord::Headers(HeadersRecord {
            immutable: true,
            response_guard: true,
            request_guard: false,
            request_no_cors: false,
            entries: response.headers,
        }),
    );
    let stream = if !response.has_body
        || matches!(
            response
                .internal_response
                .as_ref()
                .map_or(response.status, |internal| internal.status),
            0 | 204 | 205 | 304
        ) {
        None
    } else {
        Some(new_byte_stream_record(scope, state, response.body.clone())?)
    };
    let id = register_platform_object(
        state,
        PlatformObjectRecord::Response(ResponseRecord {
            timing: response.timing,
            internal_response: response.internal_response,
            body_owner: None,
            response_type: kind,
            status_text: response.status_text,
            redirected: response.redirected,
            status: response.status,
            url: response.url,
            headers_id,
            body_stream_id: stream,
            bytes: response.body,
            body_used: false,
            signal_id: None,
        }),
    );
    Ok(new_platform_object(scope, state, "Response", id)?.into())
}
fn convert<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: &Rc<V8BindingState>,
    result: CacheResult,
    single: bool,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    match result {
        CacheResult::Cache(id) => Ok(wrap(scope, state, id)?.into()),
        CacheResult::Bool(value) => Ok(v8::Boolean::new(scope, value).into()),
        CacheResult::Done => Ok(v8::undefined(scope).into()),
        CacheResult::Names(names) => {
            let array = v8::Array::new(scope, names.len() as i32);
            for (index, name) in names.iter().enumerate() {
                let name = v8::String::new_from_two_byte(scope, name, v8::NewStringType::Normal)
                    .ok_or(V8BindingError::Allocation("cache name"))?;
                array.set_index(scope, index as u32, name.into());
            }
            Ok(array.into())
        }
        CacheResult::Responses(mut responses) if single => {
            if responses.is_empty() {
                Ok(v8::undefined(scope).into())
            } else {
                response(scope, state, responses.remove(0))
            }
        }
        CacheResult::Responses(responses) => {
            let array = v8::Array::new(scope, responses.len() as i32);
            for (index, item) in responses.into_iter().enumerate() {
                let value = response(scope, state, item)?;
                array.set_index(scope, index as u32, value);
            }
            Ok(array.into())
        }
        CacheResult::Requests(requests) => {
            let array = v8::Array::new(scope, requests.len() as i32);
            for (index, request) in requests.into_iter().enumerate() {
                let input = v8::String::new(scope, &request.url)
                    .ok_or(V8BindingError::Allocation("cache request URL"))?;
                let undefined = v8::undefined(scope);
                let id = create_request_record(scope, state, input.into(), undefined.into())?;
                let headers_id = headers_from_entries(
                    state,
                    request
                        .headers
                        .into_iter()
                        .map(|(name, value)| vec![name, value])
                        .collect(),
                )?;
                if let PlatformObjectRecord::Request(record) = &mut *platform_object_mut(state, id)?
                {
                    record.method = request.method;
                    record.headers_id = headers_id;
                }
                let object = new_platform_object(scope, state, "Request", id)?;
                array.set_index(scope, index as u32, object.into());
            }
            Ok(array.into())
        }
    }
}
fn iterable<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _state: &Rc<V8BindingState>,
    value: v8::Local<'s, v8::Value>,
) -> Result<Vec<v8::Local<'s, v8::Value>>, V8BindingError> {
    let object = value.to_object(scope).ok_or_else(|| failure("TypeError"))?;
    let symbol = v8::Symbol::get_iterator(scope);
    let method = object
        .get(scope, symbol.into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .ok_or_else(|| failure("TypeError"))?;
    let mut values = Vec::new();
    super::iterable_conversion::consume(scope, object, method, |scope, value| {
        if values.len() >= 1024 {
            return Err(failure("QuotaExceededError"));
        }
        // Web IDL RequestInfo conversion happens during iteration. Genuine
        // native Requests retain identity; every other value takes the string
        // branch now, so abrupt conversion closes the still-active iterator.
        let native_request = v8::Local::<v8::Object>::try_from(value)
            .ok()
            .filter(|object| !object.is_proxy() && object.internal_field_count() > 0)
            .is_some_and(|object| {
                let Some(owner) = binding_state_for_receiver(scope, object) else {
                    return false;
                };
                let Ok(id) = stateful_id(scope, object) else {
                    return false;
                };
                let exact = owner
                    .platform_wrappers
                    .borrow()
                    .get(&id)
                    .and_then(|weak| weak.to_local(scope))
                    .is_some_and(|wrapper| wrapper.strict_equals(value));
                exact
                    && platform_object(&owner, id)
                        .is_ok_and(|record| matches!(&*record, PlatformObjectRecord::Request(_)))
            });
        let converted = if native_request {
            value
        } else {
            value
                .to_string(scope)
                .ok_or(V8BindingError::JavaScriptException)?
                .into()
        };
        values.push(converted);
        Ok(())
    })?;
    Ok(values)
}
fn retired(state: &V8BindingState) -> bool {
    state.execution_retired() || state.cache_storage.borrow().retired
}
fn defer_items(items: &[Item]) {
    for item in items {
        if let ItemState::Body(_, _, Some(lease)) = &item.state {
            let Some(owner) = lease.owner.upgrade() else {
                continue;
            };
            // Native cleanup is independent of executing author callbacks or the
            // destination realm's task queue surviving retirement.
            let stream = platform_object(&owner, lease.id)
                .ok()
                .and_then(|record| match &*record {
                    PlatformObjectRecord::ReadableStream(stream) => stream.host_stream_id,
                    _ => None,
                });
            if let Some(stream) = stream {
                if let Some(host) = owner.platform_host.borrow().as_ref() {
                    host.abandon_http_stream(stream);
                }
            }
            if !owner.execution_retired() {
                owner.deferred_platform_tasks.borrow_mut().push_back(
                    DeferredPlatformTask::CancelBodyLease {
                        lease: lease.clone(),
                    },
                );
            }
        }
    }
}
pub(super) fn cancel_lease(scope: &mut v8::PinScope, lease: BodyStreamLease) {
    v8::tc_scope!(let tc,scope);
    let reason = dom_exception_value(tc, "AbortError").unwrap_or_else(|_| v8::undefined(tc).into());
    if tc.has_caught() {
        tc.reset();
    }
    let _ = lease.cancel(tc, reason);
    if tc.has_caught() {
        tc.reset();
    }
}
fn cancel_items(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    items: &[Item],
    reason: v8::Local<v8::Value>,
) {
    for item in items {
        if let Some(signal) = item.signal {
            v8::tc_scope!(let tc,scope);
            let _ = abort_signal_with_reason(tc, state, signal, reason);
            if tc.has_caught() {
                tc.reset();
            }
        }
        if let ItemState::Body(_, _, Some(stream)) = &item.state {
            v8::tc_scope!(let tc,scope);
            let _ = stream.cancel(tc, reason);
            if tc.has_caught() {
                tc.reset();
            }
        }
    }
}
fn call<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue,
) {
    v8::tc_scope!(let scope,scope);
    let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
        return;
    };
    let Some(resolver) = v8::PromiseResolver::new(scope) else {
        return;
    };
    rv.set(resolver.get_promise(scope).into());
    let result = (|| -> Result<Option<v8::Local<v8::Value>>, V8BindingError> {
        if retired(&state) {
            return Err(failure("InvalidStateError"));
        }
        let cache = receiver(scope, &state, args.this())?;
        let raw = args.data().int32_value(scope).unwrap_or(-1);
        let storage = raw >= 100;
        let operation = if storage { raw - 100 } else { raw };
        if storage != cache.is_none() {
            return Err(failure("TypeError"));
        }
        let required = if operation == 5 {
            2
        } else if matches!(operation, 1 | 2 | 11) {
            0
        } else {
            1
        };
        if args.length() < required {
            return Err(failure("TypeError"));
        }
        if matches!(operation, 5 | 6 | 7) {
            let cache = cache.ok_or_else(|| failure("TypeError"))?;
            if {
                let bindings = state.cache_storage.borrow();
                bindings.pending.len() + bindings.in_flight >= 128
            } {
                return Err(failure("QuotaExceededError"));
            }
            let inputs = if operation == 7 {
                iterable(scope, &state, args.get(0))?
            } else {
                vec![args.get(0)]
            };
            let mut requests = Vec::new();
            // Convert and validate every Request before any network request is
            // started, so a later invalid entry cannot leak an earlier fetch.
            for input in inputs {
                let (request, id) = request_snapshot(scope, &state, input)?;
                if request.method != "GET"
                    || !url::Url::parse(&request.url)
                        .is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
                {
                    return Err(failure("TypeError"));
                }
                requests.push((request, id));
            }
            let mut items = Vec::new();
            for (request, id) in requests {
                if retired(&state) {
                    defer_items(&items);
                    return Err(failure("InvalidStateError"));
                }
                let signal = match &*platform_object(&state, id)? {
                    PlatformObjectRecord::Request(request) => Some(request.signal_id),
                    _ => None,
                };
                let next = (|| -> Result<ItemState, V8BindingError> {
                    if operation == 5 {
                        return body(scope, &state, args.get(1), false);
                    }
                    let object = new_platform_object(scope, &state, "Request", id)?;
                    let function = v8::Function::new(scope, platform_fetch_callback)
                        .ok_or(V8BindingError::Allocation("native cache fetch"))?;
                    let undefined = v8::undefined(scope);
                    let promise = function
                        .call(scope, undefined.into(), &[object.into()])
                        .and_then(|value| v8::Local::<v8::Promise>::try_from(value).ok())
                        .ok_or(V8BindingError::JavaScriptException)?;
                    Ok(ItemState::Fetch(v8::Global::new(scope, promise)))
                })();
                match next {
                    Ok(next) => items.push(Item {
                        request,
                        state: next,
                        require_ok: operation != 5,
                        signal: if operation == 5 { None } else { signal },
                    }),
                    Err(error) => {
                        let reason = dom_exception_value(scope, "AbortError")?;
                        cancel_items(scope, &state, &items, reason);
                        return Err(error);
                    }
                }
            }
            if retired(&state) {
                defer_items(&items);
                return Err(failure("InvalidStateError"));
            }
            state.cache_storage.borrow_mut().pending.push(Pending {
                cache,
                items,
                resolver: v8::Global::new(scope, resolver),
            });
            return Ok(None);
        }
        let single = operation == 0;
        let command = match operation {
            8 => CacheOperation::Open(units(scope, args.get(0))?),
            9 => CacheOperation::Has(units(scope, args.get(0))?),
            10 => CacheOperation::DeleteCache(units(scope, args.get(0))?),
            11 => CacheOperation::CacheNames,
            0 | 1 | 2 | 3 => {
                let (query, name) = query(scope, args.get(1), storage)?;
                let selected = cache;
                if storage {
                    if let Some(name) = name {
                        let request = request_snapshot(scope, &state, args.get(0))?.0;
                        return convert(
                            scope,
                            &state,
                            transport(
                                &state,
                                CacheOperation::MatchNamed {
                                    name,
                                    request,
                                    query,
                                },
                            )?,
                            true,
                        )
                        .map(Some);
                    }
                }
                let request = if args.get(0).is_undefined() && matches!(operation, 1 | 2) {
                    None
                } else {
                    Some(request_snapshot(scope, &state, args.get(0))?.0)
                };
                match operation {
                    0 => CacheOperation::Match {
                        cache: selected,
                        request: request.unwrap(),
                        query,
                    },
                    1 => CacheOperation::MatchAll {
                        cache: selected.ok_or_else(|| failure("TypeError"))?,
                        request,
                        query,
                    },
                    2 => CacheOperation::Keys {
                        cache: selected.ok_or_else(|| failure("TypeError"))?,
                        request,
                        query,
                    },
                    _ => CacheOperation::Delete {
                        cache: selected.ok_or_else(|| failure("TypeError"))?,
                        request: request.unwrap(),
                        query,
                    },
                }
            }
            _ => return Err(failure("TypeError")),
        };
        if cache.is_some() && operation >= 8 {
            return Err(failure("TypeError"));
        }
        convert(scope, &state, transport(&state, command)?, single).map(Some)
    })();
    match result {
        Ok(Some(value)) => {
            resolver.resolve(scope, value);
        }
        Ok(None) => {}
        Err(error) => {
            let exception = scope.exception();
            scope.reset();
            let value = exception.unwrap_or_else(|| {
                dom_exception_value(
                    scope,
                    match &error {
                        V8BindingError::Unsupported { member, .. } => member,
                        _ => "TypeError",
                    },
                )
                .unwrap_or_else(|_| v8::undefined(scope).into())
            });
            resolver.reject(scope, value);
        }
    }
}
fn reap(scope: &mut v8::PinScope, state: &Rc<V8BindingState>) -> Result<(), V8BindingError> {
    let queue = state.cache_storage.borrow().reclaimed.clone();
    let reclaimed = std::mem::take(&mut *queue.borrow_mut());
    let (live, pending) = {
        let mut bindings = state.cache_storage.borrow_mut();
        bindings
            .objects
            .retain(|object| object.value.to_local(scope).is_some());
        (
            bindings
                .objects
                .iter()
                .filter_map(|object| object.cache)
                .collect::<std::collections::BTreeSet<_>>(),
            bindings
                .pending
                .iter()
                .map(|pending| pending.cache)
                .collect::<std::collections::BTreeSet<_>>(),
        )
    };
    let mut failure = None;
    for id in reclaimed {
        if live.contains(&id) {
            continue;
        }
        if pending.contains(&id) {
            queue.borrow_mut().insert(id);
            continue;
        }
        if let Err(error) = transport(state, CacheOperation::ReleaseHandle { cache: id }) {
            queue.borrow_mut().insert(id);
            failure.get_or_insert(error);
        }
    }
    failure.map_or(Ok(()), Err)
}
fn poll_task(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    task: &mut Pending,
) -> Result<bool, V8BindingError> {
    let mut failed = None;
    let mut complete = true;
    for item in &mut task.items {
        if retired(state) {
            return Ok(false);
        }
        if let ItemState::Fetch(handle) = &item.state {
            let promise = v8::Local::new(scope, handle);
            match promise.state() {
                v8::PromiseState::Pending => {
                    complete = false;
                    continue;
                }
                v8::PromiseState::Rejected => {
                    failed = Some(promise.result(scope));
                    break;
                }
                v8::PromiseState::Fulfilled => {
                    let value = promise.result(scope);
                    match body(scope, state, value, item.require_ok) {
                        Ok(next) => item.state = next,
                        Err(_) => {
                            failed = Some(dom_exception_value(scope, "TypeError")?);
                            break;
                        }
                    }
                }
            }
        }
        if let ItemState::Body(metadata, handle, _) = &item.state {
            let promise = v8::Local::new(scope, handle);
            match promise.state() {
                v8::PromiseState::Pending => complete = false,
                v8::PromiseState::Rejected => {
                    failed = Some(promise.result(scope));
                    break;
                }
                v8::PromiseState::Fulfilled => {
                    let value = promise.result(scope);
                    let Ok(buffer) = v8::Local::<v8::ArrayBuffer>::try_from(value) else {
                        failed = Some(dom_exception_value(scope, "TypeError")?);
                        break;
                    };
                    if buffer.byte_length() > 32 * 1024 * 1024 {
                        failed = Some(dom_exception_value(scope, "QuotaExceededError")?);
                        break;
                    }
                    let mut response = metadata.clone();
                    response.body = buffer
                        .get_backing_store()
                        .iter()
                        .map(|byte| byte.get())
                        .collect();
                    item.state = ItemState::Done(response);
                }
            }
        }
    }
    if retired(state) {
        return Ok(false);
    }
    let resolver = v8::Local::new(scope, &task.resolver);
    if let Some(error) = failed {
        cancel_items(scope, state, &task.items, error);
        resolver.reject(scope, error);
        return Ok(true);
    }
    if complete {
        let entries = task
            .items
            .iter()
            .map(|item| match &item.state {
                ItemState::Done(response) => (item.request.clone(), response.clone()),
                _ => unreachable!(),
            })
            .collect();
        match transport(
            state,
            CacheOperation::PutAll {
                cache: task.cache,
                entries,
            },
        ) {
            Ok(_) => {
                let value = v8::undefined(scope);
                resolver.resolve(scope, value.into());
            }
            Err(error) => {
                let value = dom_exception_value(
                    scope,
                    match &error {
                        V8BindingError::Unsupported { member, .. } => member,
                        _ => "TypeError",
                    },
                )?;
                resolver.reject(scope, value);
            }
        }
        return Ok(true);
    }
    Ok(false)
}
// Allocation/conversion failures must not discard the current operation or
// unrelated operations removed from the shared queue for this checkpoint.
struct PendingBatch {
    state: Rc<V8BindingState>,
    tasks: VecDeque<Pending>,
    reserved: usize,
}
impl Drop for PendingBatch {
    fn drop(&mut self) {
        {
            let mut bindings = self.state.cache_storage.borrow_mut();
            bindings.in_flight = bindings.in_flight.saturating_sub(self.reserved);
        }
        if retired(&self.state) {
            for task in self.tasks.drain(..) {
                defer_items(&task.items);
            }
        } else if !self.tasks.is_empty() {
            self.state
                .cache_storage
                .borrow_mut()
                .pending
                .extend(self.tasks.drain(..));
        }
    }
}
pub(super) fn poll(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<usize, V8BindingError> {
    if retired(state) {
        retire(state);
        return Ok(0);
    }
    if state.execution_tasks_stopped() {
        return Ok(0);
    }
    reap(scope, state)?;
    poll_pending_with(state, |task| poll_task(scope, state, task))
}
fn poll_pending_with(
    state: &Rc<V8BindingState>,
    mut visit: impl FnMut(&mut Pending) -> Result<bool, V8BindingError>,
) -> Result<usize, V8BindingError> {
    let (tasks, reserved) = {
        let mut bindings = state.cache_storage.borrow_mut();
        let tasks = std::mem::take(&mut bindings.pending);
        let reserved = tasks.len();
        bindings.in_flight += reserved;
        (tasks.into(), reserved)
    };
    let mut batch = PendingBatch {
        state: state.clone(),
        tasks,
        reserved,
    };
    let mut count = 0;
    while !batch.tasks.is_empty() {
        if state.execution_tasks_stopped() {
            break;
        }
        let task = batch.tasks.front_mut().unwrap();
        let finished = visit(task)?;
        if retired(state) {
            break;
        }
        let task = batch.tasks.pop_front().unwrap();
        if finished {
            count += 1;
        } else {
            state.cache_storage.borrow_mut().pending.push(task);
        }
    }
    Ok(count)
}

fn illegal(scope: &mut v8::PinScope, _: v8::FunctionCallbackArguments, _: v8::ReturnValue) {
    throw_type_error(scope, "Illegal constructor")
}
fn hide_insecure_exposure(
    scope: &mut v8::PinScope,
    global: v8::Local<v8::Object>,
) -> Result<(), V8BindingError> {
    // This native installer replaces the generated caches placeholder in a
    // secure realm. Its insecure branch must remove the inherited placeholder
    // too; otherwise even typeof invokes a generic DOM receiver getter.
    let member = v8::String::new(scope, "caches")
        .ok_or(V8BindingError::Allocation("cache exposure name"))?;
    let prototype = v8::String::new(scope, "prototype")
        .ok_or(V8BindingError::Allocation("cache exposure prototype"))?;
    for owner in ["Window", "WorkerGlobalScope"] {
        let key = v8::String::new(scope, owner)
            .ok_or(V8BindingError::Allocation("cache exposure owner"))?;
        let value = global
            .get(scope, key.into())
            .ok_or(V8BindingError::JavaScriptException)?;
        if let Ok(constructor) = value.try_cast::<v8::Object>() {
            if let Some(prototype) = constructor
                .get(scope, prototype.into())
                .and_then(|value| value.try_cast::<v8::Object>().ok())
            {
                if prototype.delete(scope, member.into()) != Some(true) {
                    return Err(V8BindingError::Allocation(
                        "insecure cache prototype exposure",
                    ));
                }
            }
        }
    }
    for name in ["caches", "Cache", "CacheStorage"] {
        let key = v8::String::new(scope, name)
            .ok_or(V8BindingError::Allocation("cache exposure name"))?;
        if global.delete(scope, key.into()) != Some(true) {
            return Err(V8BindingError::Allocation("insecure cache global exposure"));
        }
    }
    Ok(())
}
pub fn install(scope: &mut v8::PinScope, state: &Rc<V8BindingState>) -> Result<(), V8BindingError> {
    let Some(descriptor) = scope
        .get_current_context()
        .get_slot::<crate::realm::RealmDescriptor>()
    else {
        return Ok(());
    };
    let secure = scope
        .get_current_context()
        .get_slot::<crate::realm::SecureContextState>()
        .map(|state| state.0)
        .unwrap_or_else(|| crate::realm::potentially_trustworthy_origin(&descriptor.origin));
    let global = scope.get_current_context().global(scope);
    if !secure {
        return hide_insecure_exposure(scope, global);
    }
    for (name, storage) in [("Cache", false), ("CacheStorage", true)] {
        let constructor = v8::Function::new(scope, illegal)
            .ok_or(V8BindingError::Allocation("cache constructor"))?;
        let key = v8::String::new(scope, "prototype").unwrap();
        let prototype = constructor
            .get(scope, key.into())
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
            .ok_or(V8BindingError::Allocation("cache prototype"))?;
        let methods: &[(&str, i32)] = if storage {
            &[
                ("match", 0),
                ("open", 8),
                ("has", 9),
                ("delete", 10),
                ("keys", 11),
            ]
        } else {
            &[
                ("match", 0),
                ("matchAll", 1),
                ("keys", 2),
                ("delete", 3),
                ("put", 5),
                ("add", 6),
                ("addAll", 7),
            ]
        };
        for (name, index) in methods {
            let data = v8::Integer::new(scope, *index + if storage { 100 } else { 0 });
            let function = v8::Function::builder(call)
                .data(data.into())
                .build(scope)
                .ok_or(V8BindingError::Allocation("cache method"))?;
            let key = v8::String::new(scope, name).unwrap();
            prototype.create_data_property(scope, key.into(), function.into());
        }
        let tag = v8::Symbol::get_to_string_tag(scope);
        let text = v8::String::new(scope, name).unwrap();
        prototype.define_own_property(
            scope,
            tag.into(),
            text.into(),
            v8::PropertyAttribute::READ_ONLY | v8::PropertyAttribute::DONT_ENUM,
        );
        let key = v8::String::new(scope, name).unwrap();
        global.create_data_property(scope, key.into(), constructor.into());
        if storage {
            let object = v8::Object::new(scope);
            object.set_prototype(scope, prototype.into());
            let handle =
                context_intrinsics::retain_object_for(scope, state, "cache-storage", object)?;
            {
                let mut bindings = state.cache_storage.borrow_mut();
                bindings.objects.push(Object {
                    cache: None,
                    value: v8::Weak::new(scope, object),
                });
                bindings.storage = Some(handle);
            }
            let key = v8::String::new(scope, "caches").unwrap();
            global.define_own_property(
                scope,
                key.into(),
                object.into(),
                v8::PropertyAttribute::READ_ONLY,
            );
        } else {
            let retained =
                context_intrinsics::retain_object_for(scope, state, "cache-prototype", prototype)?;
            state.cache_storage.borrow_mut().cache_prototype = Some(retained);
        }
    }
    Ok(())
}
pub(super) fn has_pending(state: &V8BindingState) -> bool {
    let bindings = state.cache_storage.borrow();
    let pending = !bindings.pending.is_empty() || !bindings.reclaimed.borrow().is_empty();
    pending
}
pub(super) fn retire(state: &V8BindingState) {
    let pending = {
        let mut bindings = state.cache_storage.borrow_mut();
        bindings.retired = true;
        std::mem::take(&mut bindings.pending)
    };
    for task in pending {
        defer_items(&task.items);
    }
}

#[cfg(test)]
mod foreign_request_tests {
    use super::*;
    #[test]
    fn native_secure_context_controls_cache_exposure_without_generated_getters() {
        super::super::tests::initialize_v8();
        for worker in [false, true] {
            for opaque in [false, true] {
                for secure in [false, true] {
                    let mut isolate = v8::Isolate::new(Default::default());
                    v8::scope!(let scope,&mut isolate);
                    let context = v8::Context::new(scope, Default::default());
                    let scope = &mut v8::ContextScope::new(scope, context);
                    let origin = if opaque {
                        crate::origin::Origin::opaque()
                    } else {
                        crate::origin::Origin::from_url("https://cache.test")
                    };
                    context.set_slot(Rc::new(crate::realm::RealmDescriptor {
                        id: RealmId(0),
                        origin: origin.clone(),
                        document_generation: 1,
                        frame_id: Some(1),
                    }));
                    // The host's committed ancestry/precursor result overrides the URL
                    // origin's trustworthiness; it must not change the opaque origin.
                    context.set_slot(Rc::new(crate::realm::SecureContextState(secure)));
                    let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
                    let document = dom.borrow().document();
                    let state = install_generated_bindings_with_root_for_global(
                        scope,
                        context,
                        dom,
                        RealmId(0),
                        1,
                        document,
                        if worker {
                            BindingGlobal::Worker
                        } else {
                            BindingGlobal::Window
                        },
                    )
                    .unwrap();
                    if worker {
                        install(scope, &state).unwrap();
                    }
                    assert_eq!(
                        context
                            .get_slot::<crate::realm::RealmDescriptor>()
                            .unwrap()
                            .origin,
                        origin
                    );
                    let source = if secure {
                        r#"(()=>{
                if(typeof caches!=='object'||!('caches' in globalThis))throw Error('secure caches missing');
                const descriptor=Object.getOwnPropertyDescriptor(globalThis,'caches');
                if(!descriptor||descriptor.value!==caches||caches!==globalThis.caches)throw Error('cache identity');
                if(Object.getPrototypeOf(caches)!==CacheStorage.prototype)throw Error('cache native prototype');
                return true;
            })()"#
                    } else {
                        r#"(()=>{
                if(typeof caches!=='undefined'||'caches' in globalThis)throw Error('insecure caches exposed');
                if(Object.getOwnPropertyDescriptor(globalThis,'caches'))throw Error('insecure cache descriptor');
                for(const name of ['Window','WorkerGlobalScope']){
                    const constructor=globalThis[name];
                    if(constructor&&(Object.getOwnPropertyDescriptor(constructor.prototype,'caches')||'caches' in constructor.prototype))throw Error('insecure cache prototype');
                }
                if('Cache' in globalThis||'CacheStorage' in globalThis)throw Error('insecure cache constructor');
                return true;
            })()"#
                    };
                    let source = v8::String::new(scope, source).unwrap();
                    assert!(
                        v8::Script::compile(scope, source, None)
                            .unwrap()
                            .run(scope)
                            .unwrap()
                            .is_true()
                    );
                    assert_eq!(state.cache_storage.borrow().storage.is_some(), secure);
                }
            }
        }
    }
    fn pending_body(scope: &mut v8::PinScope, cache: u64, lease: BodyStreamLease) -> Pending {
        let resolver = v8::PromiseResolver::new(scope).unwrap();
        let body = v8::PromiseResolver::new(scope).unwrap();
        Pending {
            cache,
            items: vec![Item {
                request: CacheRequest {
                    url: "https://example.test/cache".into(),
                    method: "GET".into(),
                    headers: Vec::new(),
                },
                state: ItemState::Body(
                    CacheResponse {
                        timing: None,
                        internal_response: None,
                        status: 200,
                        status_text: "OK".into(),
                        headers: Vec::new(),
                        body: Vec::new(),
                        has_body: true,
                        redirected: false,
                        url: String::new(),
                        response_type: "default".into(),
                    },
                    v8::Global::new(scope, body.get_promise(scope)),
                    Some(lease),
                ),
                require_ok: false,
                signal: None,
            }],
            resolver: v8::Global::new(scope, resolver),
        }
    }
    #[test]
    fn opaque_cache_clone_keeps_internal_bytes_private_and_replayable() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(
            scope,
            context,
            Rc::new(RefCell::new(oryn_dom::DomRuntime::new())),
            RealmId(91),
            1,
        )
        .unwrap();
        let value = response(
            scope,
            &state,
            CacheResponse {
                timing: None,
                internal_response: Some(crate::platform::InternalResponseMetadata {
                    status: 200,
                    headers: vec![("x-private".into(), "secret".into())],
                    url: "https://foreign.test/private".into(),
                    redirected: true,
                    range_requested: false,
                }),
                status: 0,
                status_text: String::new(),
                headers: Vec::new(),
                body: b"private bytes".to_vec(),
                has_body: true,
                redirected: false,
                url: String::new(),
                response_type: "opaque".into(),
            },
        )
        .unwrap();
        let key = v8::String::new(scope, "opaqueResponse").unwrap();
        context.global(scope).set(scope, key.into(), value);
        let source=v8::String::new(scope,"if(opaqueResponse.status!==0||opaqueResponse.url!==''||opaqueResponse.body!==null||opaqueResponse.bodyUsed||opaqueResponse.headers.has('x-private'))throw Error('private metadata leak');globalThis.opaqueClone=opaqueResponse.clone();opaqueResponse.text().then(t=>{if(t!=='')throw Error('private byte leak')});").unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        scope.perform_microtask_checkpoint();
        let key = v8::String::new(scope, "opaqueClone").unwrap();
        let clone = context.global(scope).get(scope, key.into()).unwrap();
        let ItemState::Body(metadata, promise, _) = body(scope, &state, clone, false).unwrap()
        else {
            panic!("cache body")
        };
        assert_eq!(metadata.internal_response.as_ref().unwrap().status, 200);
        scope.perform_microtask_checkpoint();
        let promise = v8::Local::new(scope, promise);
        assert_eq!(promise.state(), v8::PromiseState::Fulfilled);
        let result = promise.result(scope);
        let buffer = v8::Local::<v8::ArrayBuffer>::try_from(result).unwrap();
        assert_eq!(bytes_from_array_buffer(buffer), b"private bytes");
        assert!(
            body(scope, &state, value, true).is_err(),
            "Cache.add success check still sees filtered status"
        );
    }
    #[test]
    fn retirement_transfers_foreign_lease_and_never_cancels_local_decoy() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let (source, lease, id) = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
            let state = install_generated_bindings(scope, first, dom, RealmId(84), 1).unwrap();
            let id = new_byte_stream_record(scope, &state, vec![1]).unwrap();
            let lease = BodyStreamLease::capture(scope, &state, id).unwrap();
            (state, lease, id)
        };
        let second = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, second);
        let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
        let destination = install_generated_bindings(scope, second, dom, RealmId(85), 1).unwrap();
        let decoy = new_byte_stream_record(scope, &destination, vec![2]).unwrap();
        assert_eq!(id, decoy);
        assert!(
            matches!(&*platform_object(&source,id).unwrap(),PlatformObjectRecord::ReadableStream(stream) if stream.bytes.as_ref().unwrap().close_requested&&!stream.closed)
        );
        destination
            .cache_storage
            .borrow_mut()
            .pending
            .push(pending_body(scope, 1, lease));
        retire(&destination);
        retire(&destination);
        assert!(destination.cache_storage.borrow().pending.is_empty());
        let task = source
            .deferred_platform_tasks
            .borrow_mut()
            .pop_front()
            .unwrap();
        let DeferredPlatformTask::CancelBodyLease { lease } = task else {
            panic!("wrong cleanup route")
        };
        cancel_lease(scope, lease);
        assert!(scope.get_current_context() == second);
        assert!(source.deferred_platform_tasks.borrow().is_empty());
        assert!(
            matches!(&*platform_object(&source,id).unwrap(),PlatformObjectRecord::ReadableStream(stream) if stream.disturbed&&stream.closed)
        );
        assert!(
            matches!(&*platform_object(&source,id).unwrap(),PlatformObjectRecord::ReadableStream(stream) if stream.queue.is_empty()&&stream.bytes.as_ref().unwrap().pending.is_empty())
        );
        assert!(
            matches!(&*platform_object(&destination,decoy).unwrap(),PlatformObjectRecord::ReadableStream(stream) if !stream.disturbed&&!stream.closed)
        );
    }
    #[test]
    fn callback_retirement_cleans_current_and_unvisited_without_revival() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
        let state = install_generated_bindings(scope, context, dom, RealmId(86), 1).unwrap();
        for cache in [1, 2] {
            let id = new_byte_stream_record(scope, &state, vec![1]).unwrap();
            let lease = BodyStreamLease::capture(scope, &state, id).unwrap();
            state
                .cache_storage
                .borrow_mut()
                .pending
                .push(pending_body(scope, cache, lease));
        }
        let mut visits = 0;
        assert!(
            poll_pending_with(&state, |_| {
                visits += 1;
                assert_eq!(state.cache_storage.borrow().in_flight, 2);
                retire(&state);
                Err(V8BindingError::Allocation("retired during conversion"))
            })
            .is_err()
        );
        assert_eq!(visits, 1);
        assert!(state.cache_storage.borrow().pending.is_empty());
        assert_eq!(state.cache_storage.borrow().in_flight, 0);
        assert_eq!(
            state
                .deferred_platform_tasks
                .borrow()
                .iter()
                .filter(|task| matches!(task, DeferredPlatformTask::CancelBodyLease { .. }))
                .count(),
            2
        );
        assert_eq!(
            poll_pending_with(&state, |_| panic!("retired checkpoint visited work")).unwrap(),
            0
        );
    }
    #[test]
    fn failed_first_body_cleanup_does_not_skip_later_stream() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
        let state = install_generated_bindings(scope, context, dom, RealmId(87), 1).unwrap();
        let first = new_byte_stream_record(scope, &state, vec![1]).unwrap();
        let second = new_byte_stream_record(scope, &state, vec![2]).unwrap();
        let first_lease = BodyStreamLease::capture(scope, &state, first).unwrap();
        let second_lease = BodyStreamLease::capture(scope, &state, second).unwrap();
        if let PlatformObjectRecord::ReadableStream(stream) =
            &mut *platform_object_mut(&state, first).unwrap()
        {
            stream.controller_id = u64::MAX;
        }
        let mut task = pending_body(scope, 1, first_lease);
        task.items
            .extend(pending_body(scope, 2, second_lease).items);
        task.items[0].signal = Some(u64::MAX);
        let reason = dom_exception_value(scope, "AbortError").unwrap();
        cancel_items(scope, &state, &task.items, reason);
        assert!(
            matches!(&*platform_object(&state,second).unwrap(),PlatformObjectRecord::ReadableStream(stream) if stream.disturbed&&stream.closed)
        );
    }
    #[test]
    fn failed_cache_checkpoint_retains_current_and_unvisited_operations() {
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
        let state = install_generated_bindings(scope, context, dom, RealmId(83), 1).unwrap();
        let mut promises = Vec::new();
        for cache in [10, 20, 30] {
            let resolver = v8::PromiseResolver::new(scope).unwrap();
            promises.push(v8::Global::new(scope, resolver.get_promise(scope)));
            state.cache_storage.borrow_mut().pending.push(Pending {
                cache,
                items: Vec::new(),
                resolver: v8::Global::new(scope, resolver),
            });
        }
        let mut visited = Vec::new();
        assert!(
            poll_pending_with(&state, |task| {
                visited.push(task.cache);
                if task.cache == 20 {
                    return Err(V8BindingError::Allocation("injected conversion failure"));
                }
                let value = v8::undefined(scope);
                v8::Local::new(scope, &task.resolver).resolve(scope, value.into());
                Ok(true)
            })
            .is_err()
        );
        assert_eq!(visited, vec![10, 20]);
        assert_eq!(
            state
                .cache_storage
                .borrow()
                .pending
                .iter()
                .map(|task| task.cache)
                .collect::<Vec<_>>(),
            vec![20, 30]
        );
        assert_eq!(
            v8::Local::new(scope, &promises[0]).state(),
            v8::PromiseState::Fulfilled
        );
        for promise in &promises[1..] {
            assert_eq!(
                v8::Local::new(scope, promise).state(),
                v8::PromiseState::Pending
            );
        }
        assert_eq!(
            poll_pending_with(&state, |task| {
                let value = v8::undefined(scope);
                v8::Local::new(scope, &task.resolver).resolve(scope, value.into());
                Ok(true)
            })
            .unwrap(),
            2
        );
        assert!(state.cache_storage.borrow().pending.is_empty());
        for promise in promises {
            assert_eq!(
                v8::Local::new(scope, promise).state(),
                v8::PromiseState::Fulfilled
            );
        }
    }
    #[test]
    fn cache_request_sequence_preserves_foreign_brand_and_conversion_order() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let first = v8::Context::new(scope, Default::default());
        let (_source, request) = {
            let scope = &mut v8::ContextScope::new(scope, first);
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            let state = install_generated_bindings(scope, first, dom, RealmId(81), 1).unwrap();
            let code=v8::String::new(scope,"const r=new Request('https://example.test/foreign',{headers:{'x-owner':'foreign'}});r[Symbol.toPrimitive]=()=>{throw Error('Request must not be coerced')};r").unwrap();
            let value = v8::Script::compile(scope, code, None)
                .unwrap()
                .run(scope)
                .unwrap();
            (state, v8::Global::new(scope, value))
        };
        let second = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, second);
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<body></body>")
                .into_runtime()
                .unwrap(),
        ));
        let destination = install_generated_bindings(scope, second, dom, RealmId(82), 1).unwrap();
        let foreign = v8::Local::new(scope, &request);
        let key = v8::String::new(scope, "foreign").unwrap();
        second
            .global(scope)
            .set(scope, key.into(), foreign)
            .unwrap();
        let code=v8::String::new(scope,"globalThis.order=[];[foreign,{toString(){order.push('string');return 'https://example.test/plain'}}]").unwrap();
        let input = v8::Script::compile(scope, code, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let values = iterable(scope, &destination, input).unwrap();
        assert_eq!(values.len(), 2);
        assert!(values[0].strict_equals(foreign));
        assert_eq!(
            values[1].to_rust_string_lossy(scope),
            "https://example.test/plain"
        );
        let (snapshot, _) = request_snapshot(scope, &destination, values[0]).unwrap();
        assert_eq!(snapshot.url, "https://example.test/foreign");
        assert!(
            snapshot
                .headers
                .iter()
                .any(|(name, value)| name == "x-owner" && value == "foreign")
        );
        let code = v8::String::new(scope, "order.join(',')").unwrap();
        assert_eq!(
            v8::Script::compile(scope, code, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .to_rust_string_lossy(scope),
            "string"
        );
    }
}

#[cfg(test)]
mod retention_tests {
    use super::*;
    #[test]
    fn secure_window_and_worker_cache_identity_does_not_root_retired_context() {
        super::super::tests::initialize_v8();
        for worker in [false, true] {
            let mut isolate = v8::Isolate::new(Default::default());
            let (context, state, anchor) = {
                v8::scope!(let handles, &mut isolate);
                let context = v8::Context::new(handles, Default::default());
                let scope = &mut v8::ContextScope::new(handles, context);
                context.set_slot(Rc::new(crate::realm::RealmDescriptor {
                    id: RealmId(994),
                    origin: crate::origin::Origin::from_url("https://cache.test"),
                    document_generation: 1,
                    frame_id: None,
                }));
                context.set_slot(Rc::new(crate::realm::SecureContextState(true)));
                let dom = Rc::new(RefCell::new(
                    oryn_dom::html::parse("<body></body>")
                        .into_runtime()
                        .unwrap(),
                ));
                let document = dom.borrow().document();
                let state = install_generated_bindings_with_root_for_global(
                    scope,
                    context,
                    dom,
                    RealmId(994),
                    1,
                    document,
                    if worker {
                        BindingGlobal::Worker
                    } else {
                        BindingGlobal::Window
                    },
                )
                .unwrap();
                if worker {
                    install(scope, &state).unwrap();
                }
                let original = state
                    .cache_storage
                    .borrow()
                    .cache_prototype
                    .clone()
                    .unwrap();
                let storage = state.cache_storage.borrow().storage.clone().unwrap();
                assert!(
                    super::super::test_eval::eval(scope, "caches")
                        .strict_equals(storage.to_local(scope).unwrap().into())
                );
                super::super::test_eval::eval(
                    scope,
                    "Cache=function replacement(){};CacheStorage=function replacement(){};",
                );
                let cache = wrap(scope, &state, 19).unwrap();
                assert!(
                    cache
                        .get_prototype(scope)
                        .unwrap()
                        .strict_equals(original.to_local(scope).unwrap().into())
                );
                state.retire_execution();
                state.refresh_connected_wrapper_roots(scope);
                (
                    v8::Weak::new(scope, context),
                    state,
                    v8::Global::new(scope, cache),
                )
            };
            isolate.low_memory_notification();
            isolate.low_memory_notification();
            {
                v8::scope!(let handles, &mut isolate);
                assert!(context.to_local(handles).is_some());
                assert!(
                    state
                        .cache_storage
                        .borrow()
                        .storage
                        .as_ref()
                        .unwrap()
                        .to_local(handles)
                        .is_some()
                );
            }
            drop(anchor);
            isolate.low_memory_notification();
            isolate.low_memory_notification();
            v8::scope!(let handles, &mut isolate);
            assert!(context.to_local(handles).is_none());
            assert!(
                state
                    .cache_storage
                    .borrow()
                    .cache_prototype
                    .as_ref()
                    .unwrap()
                    .to_local(handles)
                    .is_none()
            );
            assert!(
                state
                    .cache_storage
                    .borrow()
                    .storage
                    .as_ref()
                    .unwrap()
                    .to_local(handles)
                    .is_none()
            );
        }
    }
}
