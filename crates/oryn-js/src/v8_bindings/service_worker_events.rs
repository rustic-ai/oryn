//! Service worker event lifetimes and native fetch response extraction.
use super::*;
use crate::worker_wire::{
    AgentCommand, AgentEvent, AgentFetchRequest, AgentFetchResponse, LifecycleEvent, WorkerError,
};
const LIMIT: usize = 32 * 1024 * 1024;
#[derive(Default)]
pub(super) struct Events {
    pending: BTreeMap<u64, Pending>,
    ready: Vec<AgentEvent>,
}
struct Pending {
    preload_signal: Option<u64>,
    task: u64,
    event: v8::Global<v8::Object>,
    dispatching: bool,
    wait: Vec<v8::Global<v8::Promise>>,
    pending_count: usize,
    rejected: bool,
    request: Option<AgentFetchRequest>,
    response: Option<v8::Global<v8::Promise>>,
    body: Option<(
        v8::Global<v8::Promise>,
        AgentFetchResponse,
        Option<BodyStreamLease>,
    )>,
    responded: bool,
    response_done: bool,
    handled: Option<v8::Global<v8::PromiseResolver>>,
    stream: Option<service_body::Reader>,
    stream_started: bool,
    credits: u8,
}
// Cleanup callbacks may throw or return native errors. Isolate each attempt so
// one source cannot suppress another source's release or the terminal receipt.
macro_rules! cleanup_attempt {
    ($scope:expr, |$tc:ident| $body:expr) => {{
        v8::tc_scope!(let $tc, $scope);
        let result:Result<(),V8BindingError>=($body).map(|_|());
        let failed=result.is_err()||$tc.has_caught();
        if $tc.has_caught(){$tc.reset();}
        failed
    }};
}
fn error(scope: &mut v8::PinScope, name: &str) -> Result<(), V8BindingError> {
    let error = dom_exception_value(scope, name)?;
    scope.throw_exception(error);
    Err(V8BindingError::JavaScriptException)
}
fn failure(message: &str) -> WorkerError {
    WorkerError::new("NetworkError", message)
}
fn fetch_failure(message: &str) -> WorkerError {
    failure(message).with_script_failure(crate::worker_wire::ScriptFailureKind::Fetch)
}
fn promise(
    scope: &mut v8::PinScope,
    value: v8::Local<v8::Value>,
) -> Result<v8::Global<v8::Promise>, V8BindingError> {
    let resolver =
        v8::PromiseResolver::new(scope).ok_or(V8BindingError::Allocation("service promise"))?;
    resolver
        .resolve(scope, value)
        .ok_or(V8BindingError::JavaScriptException)?;
    Ok(v8::Global::new(scope, resolver.get_promise(scope)))
}
pub(super) fn initialize<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _state: &Rc<V8BindingState>,
    object: v8::Local<'s, v8::Object>,
    interface: &str,
    init: Option<v8::Local<'s, v8::Object>>,
) -> Result<(), V8BindingError> {
    let fields = v8::Array::new(scope, 10);
    for (name, index) in [
        ("clientId", 1),
        ("resultingClientId", 2),
        ("replacesClientId", 3),
    ] {
        let value =
            event_init_member(scope, init, name).ok_or(V8BindingError::JavaScriptException)?;
        let value = if value.is_undefined() {
            v8::String::new(scope, "").ok_or(V8BindingError::Allocation("fetch event string"))?
        } else {
            value
                .to_string(scope)
                .ok_or(V8BindingError::JavaScriptException)?
        };
        fields.set_index(scope, index, value.into());
    }
    if interface == "FetchEvent" {
        let request =
            event_init_member(scope, init, "request").ok_or(V8BindingError::JavaScriptException)?;
        let valid = v8::Local::<v8::Object>::try_from(request)
            .ok()
            .filter(|object| !object.is_proxy())
            .and_then(|object| {
                binding_state_for_receiver(scope, object).map(|owner| (owner, object))
            })
            .is_some_and(|(owner, object)| {
                stateful_id(scope, object).is_ok_and(|id| {
                    matches!(
                        owner.platform_objects.borrow().get(&id),
                        Some(PlatformObjectRecord::Request(_))
                    )
                })
            });
        if !valid {
            throw_type_error(scope, "FetchEventInit requires a Request");
            return Err(V8BindingError::JavaScriptException);
        }
        fields.set_index(scope, 0, request);
    } else {
        let value =
            event_init_member(scope, init, "data").ok_or(V8BindingError::JavaScriptException)?;
        fields.set_index(scope, 6, value);
        for (name, index) in [("origin", 7), ("lastEventId", 8)] {
            let value =
                event_init_member(scope, init, name).ok_or(V8BindingError::JavaScriptException)?;
            let value = if value.is_undefined() {
                v8::String::new(scope, "")
                    .ok_or(V8BindingError::Allocation("service message string"))?
            } else {
                value
                    .to_string(scope)
                    .ok_or(V8BindingError::JavaScriptException)?
            };
            fields.set_index(scope, index, value.into());
        }
        let source =
            event_init_member(scope, init, "source").ok_or(V8BindingError::JavaScriptException)?;
        let source = if source.is_undefined() {
            v8::null(scope).into()
        } else {
            source
        };
        fields.set_index(scope, 9, source);
        let ports =
            event_init_member(scope, init, "ports").ok_or(V8BindingError::JavaScriptException)?;
        let options = v8::Object::new(scope);
        let key = v8::String::new(scope, "transfer")
            .ok_or(V8BindingError::Allocation("message ports conversion"))?;
        options.create_data_property(scope, key.into(), ports);
        let ports = structured_clone::transfer_list(scope, options.into())
            .ok_or(V8BindingError::JavaScriptException)?;
        for value in &ports {
            let valid = v8::Local::<v8::Object>::try_from(*value)
                .ok()
                .and_then(|object| {
                    binding_state_for_receiver(scope, object).map(|owner| (owner, object))
                })
                .is_some_and(|(owner, object)| {
                    stateful_id(scope, object).is_ok_and(|id| {
                        matches!(
                            owner.platform_objects.borrow().get(&id),
                            Some(
                                PlatformObjectRecord::MessagePort(_)
                                    | PlatformObjectRecord::DetachedMessagePort
                            )
                        )
                    })
                });
            if !valid {
                throw_type_error(
                    scope,
                    "ExtendableMessageEvent ports require MessagePort values",
                );
                return Err(V8BindingError::JavaScriptException);
            }
        }
        let ports = v8::Array::new_with_elements(scope, &ports);
        ports.set_integrity_level(scope, v8::IntegrityLevel::Frozen);
        fields.set_index(scope, 0, ports.into());
    }
    let undefined = v8::undefined(scope);
    let preload = resolved_promise(scope, undefined.into())?;
    fields.set_index(scope, 4, preload.into());
    let handled = v8::PromiseResolver::new(scope)
        .ok_or(V8BindingError::Allocation("fetch handled promise"))?;
    fields.set_index(scope, 5, handled.get_promise(scope).into());
    object.set_internal_field(2, fields.into());
    Ok(())
}
pub fn begin(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    command: AgentCommand,
) -> Result<(), V8BindingError> {
    if state.execution_retired() {
        return Err(V8BindingError::Allocation("retired service worker"));
    }
    let res = {
        let events = state.service_events.borrow();
        events.pending.len() + events.ready.len() >= 1024
    };
    if res {
        return error(scope, "QuotaExceededError");
    }
    let (task, event_type, request, payload, source) = match command {
        AgentCommand::Fetch(request) => (request.id, "fetch", Some(request), None, None),
        AgentCommand::ServiceMessage { id, source, value } => {
            (id, "message", None, Some(value), Some(source))
        }
        AgentCommand::Lifecycle { id, event, payload } => (
            id,
            match event {
                LifecycleEvent::Install => "install",
                LifecycleEvent::Activate => "activate",
                LifecycleEvent::Message => "message",
                LifecycleEvent::Push => "push",
                LifecycleEvent::Sync => "sync",
            },
            None,
            payload,
            None,
        ),
        _ => return Err(V8BindingError::Allocation("service command kind")),
    };
    if state
        .service_events
        .borrow()
        .pending
        .values()
        .any(|pending| pending.task == task)
    {
        return Err(V8BindingError::Allocation(
            "duplicate service task identity",
        ));
    }
    let interface = if request.is_some() {
        "FetchEvent"
    } else if event_type == "message" {
        "ExtendableMessageEvent"
    } else {
        "ExtendableEvent"
    };
    let event = create_event_object_with_interface(
        scope,
        state,
        interface,
        event_type,
        false,
        request.is_some(),
    )?;
    let id = event_id(scope, event)?;
    if let Some(record) = state.events.borrow_mut().get_mut(&id) {
        record.is_trusted = true;
    }
    let fields = v8::Array::new(scope, 10);
    let empty =
        v8::String::new(scope, "").ok_or(V8BindingError::Allocation("service empty string"))?;
    for index in 1..4 {
        fields.set_index(scope, index, empty.into());
    }
    fields.set_index(scope, 7, empty.into());
    fields.set_index(scope, 8, empty.into());
    let null = v8::null(scope);
    fields.set_index(scope, 9, null.into());
    let undefined = v8::undefined(scope);
    if event_type == "message" {
        let ports = v8::Array::new(scope, 0);
        ports.set_integrity_level(scope, v8::IntegrityLevel::Frozen);
        fields.set_index(scope, 0, ports.into());
        fields.set_index(scope, 6, null.into());
    }
    let preload_signal = if request
        .as_ref()
        .is_some_and(|request| request.navigation_preload)
    {
        let resolver = v8::PromiseResolver::new(scope)
            .ok_or(V8BindingError::Allocation("navigation preload promise"))?;
        let signal = register_platform_object(
            state,
            PlatformObjectRecord::AbortSignal(AbortSignalRecord {
                aborted: false,
                reason: None,
                followers: Vec::new(),
                foreign_followers: Vec::new(),
                foreign_sources: Vec::new(),
            }),
        );
        fields.set_index(scope, 4, resolver.get_promise(scope).into());
        resolver.get_promise(scope).mark_as_handled();
        submit_platform_operation(
            state,
            PlatformHostRequest::NavigationPreload { task },
            PendingPlatformOperation::FetchStart {
                head_request: false,
                resolver: v8::Global::new(scope, resolver),
                signal_id: signal,
            },
        )?;
        Some(signal)
    } else {
        let preload = resolved_promise(scope, undefined.into())?;
        fields.set_index(scope, 4, preload.into());
        None
    };
    let handled = if let Some(request) = &request {
        if request.body.len() > LIMIT {
            return error(scope, "QuotaExceededError");
        }
        let headers_id = register_platform_object(
            state,
            PlatformObjectRecord::Headers(HeadersRecord {
                immutable: true,
                request_guard: true,
                request_no_cors: false,
                response_guard: false,
                entries: request.headers.clone(),
            }),
        );
        let signal_id = register_platform_object(
            state,
            PlatformObjectRecord::AbortSignal(AbortSignalRecord {
                aborted: false,
                reason: None,
                followers: Vec::new(),
                foreign_followers: Vec::new(),
                foreign_sources: Vec::new(),
            }),
        );
        let body_object_id = if request.body.is_empty() {
            None
        } else {
            Some(new_byte_stream_record(scope, state, request.body.clone())?)
        };
        let request_id = register_platform_object(
            state,
            PlatformObjectRecord::Request(RequestRecord {
                blob_entry: None,
                keepalive: request.keepalive,
                body_source_known: true,
                privileged_range: request.privileged_range,
                cache: request.cache,
                navigation: request.navigation,
                integrity: request.integrity.clone(),
                body_owner: None,
                mode: request.mode.clone(),
                redirect: request.redirect.clone(),
                destination: request.destination.clone(),
                url: request.url.clone(),
                method: request.method.clone(),
                headers_id,
                signal_id,
                credentials: request.credentials.clone(),
                referrer: String::new(),
                referrer_policy: String::new(),
                body: request.body.clone(),
                body_object_id,
                body_used: false,
            }),
        );
        let object = new_platform_object(scope, state, "Request", request_id)?;
        fields.set_index(scope, 0, object.into());
        let client = v8::String::new(scope, &request.client_id)
            .ok_or(V8BindingError::Allocation("fetch client"))?;
        fields.set_index(scope, 1, client.into());
        let resolver =
            v8::PromiseResolver::new(scope).ok_or(V8BindingError::Allocation("fetch handled"))?;
        fields.set_index(scope, 5, resolver.get_promise(scope).into());
        Some(v8::Global::new(scope, resolver))
    } else {
        None
    };
    if let Some(payload) = payload {
        let (value, ports) = structured_clone::deserialize_wire(scope, payload)
            .ok_or(V8BindingError::Allocation("service message clone"))?;
        fields.set_index(scope, 6, value);
        let ports = ports
            .into_iter()
            .map(Into::into)
            .collect::<Vec<v8::Local<v8::Value>>>();
        let ports = v8::Array::new_with_elements(scope, &ports);
        ports.set_integrity_level(scope, v8::IntegrityLevel::Frozen);
        fields.set_index(scope, 0, ports.into());
    }
    if let Some(source) = source {
        let (object, origin) = service_workers::message_source(scope, state, source)?;
        fields.set_index(scope, 9, object);
        let origin = v8_string(scope, &origin)?;
        fields.set_index(scope, 7, origin);
    }
    event.set_internal_field(2, fields.into());
    state.service_events.borrow_mut().pending.insert(
        id,
        Pending {
            preload_signal,
            task,
            event: v8::Global::new(scope, event),
            dispatching: true,
            wait: Vec::new(),
            pending_count: 0,
            rejected: false,
            request,
            response: None,
            body: None,
            responded: false,
            response_done: false,
            handled,
            stream: None,
            stream_started: false,
            credits: 0,
        },
    );
    let result = global_events::dispatch(scope, state, event).map(|_| ());
    if let Some(pending) = state.service_events.borrow_mut().pending.get_mut(&id) {
        pending.dispatching = false;
    }
    result
}
pub(super) fn operation(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    object: v8::Local<v8::Object>,
    member: &str,
    args: &v8::FunctionCallbackArguments,
) -> Result<(), V8BindingError> {
    let id = event_id(scope, object)?;
    if args.length() == 0 {
        throw_type_error(scope, "service event method requires a promise");
        return Err(V8BindingError::JavaScriptException);
    }
    let promise = promise(scope, args.get(0))?;
    let allowed = {
        let events = state.service_events.borrow();
        events.pending.get(&id).is_some_and(|pending| match member {
            "waitUntil" => pending.dispatching || pending.pending_count > 0,
            "respondWith" => pending.dispatching && pending.request.is_some() && !pending.responded,
            _ => false,
        })
    };
    if !allowed {
        return error(scope, "InvalidStateError");
    }
    if state.service_events.borrow().pending[&id].wait.len() >= 1024 {
        return error(scope, "QuotaExceededError");
    }
    {
        let mut events = state.service_events.borrow_mut();
        let pending = events
            .pending
            .get_mut(&id)
            .ok_or(V8BindingError::Allocation(
                "service event retired during conversion",
            ))?;
        if member == "respondWith" {
            pending.responded = true;
            pending.response = Some(promise.clone());
            if let Some(record) = state.events.borrow_mut().get_mut(&id) {
                record.propagation_stopped = true;
                record.immediate_stopped = true;
            }
        }
        pending.pending_count += 1;
        pending.wait.push(promise.clone());
    }
    let event_id = v8::Number::new(scope, id as f64);
    let no = v8::Boolean::new(scope, false);
    let yes = v8::Boolean::new(scope, true);
    let success_data = v8::Array::new_with_elements(scope, &[event_id.into(), no.into()]);
    let failure_data = v8::Array::new_with_elements(scope, &[event_id.into(), yes.into()]);
    let success = v8::Function::builder(wait_settled)
        .data(success_data.into())
        .build(scope)
        .ok_or(V8BindingError::Allocation("service wait reaction"))?;
    let failure = v8::Function::builder(wait_settled)
        .data(failure_data.into())
        .build(scope)
        .ok_or(V8BindingError::Allocation("service wait reaction"))?;
    v8::Local::new(scope, &promise)
        .then2(scope, success, failure)
        .ok_or(V8BindingError::JavaScriptException)?;
    Ok(())
}
fn wait_settled(scope: &mut v8::PinScope, args: v8::FunctionCallbackArguments, _: v8::ReturnValue) {
    let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() else {
        return;
    };
    let Ok(data) = v8::Local::<v8::Array>::try_from(args.data()) else {
        return;
    };
    let Some(id) = data
        .get_index(scope, 0)
        .and_then(|value| value.number_value(scope))
        .map(|value| value as u64)
    else {
        return;
    };
    let rejected = data
        .get_index(scope, 1)
        .is_some_and(|value| value.boolean_value(scope));
    if let Some(pending) = state.service_events.borrow_mut().pending.get_mut(&id) {
        pending.pending_count = pending.pending_count.saturating_sub(1);
        pending.rejected |= rejected;
    }
}
pub(super) fn attribute<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _state: &V8BindingState,
    object: v8::Local<'s, v8::Object>,
    member: &str,
) -> Result<v8::Local<'s, v8::Value>, V8BindingError> {
    event_id(scope, object)?;
    let fields = object
        .get_internal_field(scope, 2)
        .and_then(|value| v8::Local::<v8::Value>::try_from(value).ok())
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .ok_or(V8BindingError::Allocation("service event fields"))?;
    let index = match member {
        "request" | "ports" => 0,
        "clientId" => 1,
        "resultingClientId" => 2,
        "replacesClientId" => 3,
        "preloadResponse" => 4,
        "handled" => 5,
        "data" => 6,
        "origin" => 7,
        "lastEventId" => 8,
        "source" => 9,
        _ => return Err(V8BindingError::Allocation("service event attribute")),
    };
    fields
        .get_index(scope, index)
        .ok_or(V8BindingError::Allocation("service event attribute"))
}
fn extract_response(
    scope: &mut v8::PinScope,
    _state: &Rc<V8BindingState>,
    value: v8::Local<v8::Value>,
    request: &AgentFetchRequest,
) -> Result<
    (
        v8::Global<v8::Promise>,
        AgentFetchResponse,
        Option<BodyStreamLease>,
    ),
    V8BindingError,
> {
    let object = v8::Local::<v8::Object>::try_from(value)
        .map_err(|_| V8BindingError::Allocation("respondWith requires Response"))?;
    if object.is_proxy() {
        return Err(V8BindingError::Allocation(
            "respondWith requires native Response",
        ));
    }
    let owner = binding_state_for_receiver(scope, object)
        .ok_or(V8BindingError::Allocation("Response native owner"))?;
    let state = &owner;
    let id = stateful_id(scope, object)?;
    if !state
        .platform_wrappers
        .borrow()
        .get(&id)
        .and_then(|weak| weak.to_local(scope))
        .is_some_and(|wrapper| wrapper.strict_equals(value))
    {
        return Err(V8BindingError::Allocation("Response ownership"));
    }
    let (metadata, stream, foreign) = {
        let record = platform_object(state, id)?;
        let PlatformObjectRecord::Response(response) = &*record else {
            return Err(V8BindingError::Allocation("respondWith requires Response"));
        };
        if response.response_type == "error"
            || (response.response_type == "opaque" && request.mode != "no-cors")
            || (response.response_type == "opaqueredirect" && request.redirect != "manual")
        {
            return Err(V8BindingError::Allocation(
                "fetch response type does not match request",
            ));
        }
        let headers = match &*platform_object(state, response.headers_id)? {
            PlatformObjectRecord::Headers(headers) => headers.entries.clone(),
            _ => return Err(V8BindingError::Allocation("Response headers")),
        };
        {
            let internal = response.internal_response.as_ref();
            if response.response_type == "opaque"
                && internal
                    .is_some_and(|internal| internal.status == 206 && internal.range_requested)
                && !request
                    .headers
                    .iter()
                    .any(|(name, _)| name.eq_ignore_ascii_case("range"))
            {
                return Err(V8BindingError::Allocation(
                    "opaque ranged response requires a range request",
                ));
            }
            (
                AgentFetchResponse {
                    timing: response.timing.clone(),
                    redirected: internal
                        .map_or(response.redirected, |internal| internal.redirected),
                    range_requested: internal.is_some_and(|internal| internal.range_requested),
                    status: internal.map_or(response.status, |internal| internal.status),
                    status_text: response.status_text.clone(),
                    headers: internal.map_or(headers, |internal| internal.headers.clone()),
                    body: Vec::new(),
                    response_url: internal
                        .map_or_else(|| response.url.clone(), |internal| internal.url.clone()),
                    response_type: response.response_type.into(),
                },
                response.body_stream_id,
                response.body_owner.clone(),
            )
        }
    };
    let stream = stream
        .map(|id| BodyStreamLease::capture(scope, &body_owner(state, &foreign)?, id))
        .transpose()?;
    let value =
        response_body_operation_internal(scope, state, id, "arrayBuffer", Some(LIMIT), true)?;
    let promise = v8::Local::<v8::Promise>::try_from(value)
        .map_err(|_| V8BindingError::Allocation("Response body promise"))?;
    Ok((v8::Global::new(scope, promise), metadata, stream))
}
pub fn poll(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
) -> Result<Vec<AgentEvent>, V8BindingError> {
    let mut output = std::mem::take(&mut state.service_events.borrow_mut().ready);
    match poll_inner(scope, state, &mut output) {
        Ok(()) => Ok(output),
        Err(error) => {
            // Earlier events may already have committed response_done/removal.
            // A later failure must not discard their only terminal receipts.
            state.service_events.borrow_mut().ready.extend(output);
            Err(error)
        }
    }
}
fn poll_inner(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    output: &mut Vec<AgentEvent>,
) -> Result<(), V8BindingError> {
    let ids = state
        .service_events
        .borrow()
        .pending
        .keys()
        .copied()
        .collect::<Vec<_>>();
    for id in ids {
        // Snapshot handles without holding the event-table borrow across author
        // callbacks. Revalidate ownership before publishing their result.
        let (task, request, response, body, responded, response_done) = {
            let events = state.service_events.borrow();
            let Some(pending) = events.pending.get(&id) else {
                continue;
            };
            (
                pending.task,
                pending.request.clone(),
                pending.response.clone(),
                pending.body.clone(),
                pending.responded,
                pending.response_done,
            )
        };
        let streaming = request.as_ref().is_some_and(|request| request.streaming) && responded;
        if !response_done && streaming {
            poll_streaming(scope, state, id, output)?;
        }
        if !response_done
            && !streaming
            && let Some(request) = request
        {
            let mut result = None;
            if !responded {
                output.push(AgentEvent::FetchFallback { id: task });
                result = Some(Ok(None));
            } else if let Some((promise, metadata, _)) = body {
                let promise = v8::Local::new(scope, &promise);
                match promise.state() {
                    v8::PromiseState::Pending => {}
                    v8::PromiseState::Rejected => {
                        result = Some(Err(fetch_failure("service response body rejected")))
                    }
                    v8::PromiseState::Fulfilled => {
                        let value = promise.result(scope);
                        let bytes = v8::Local::<v8::ArrayBuffer>::try_from(value).ok();
                        result = Some(match bytes {
                            Some(buffer) if buffer.byte_length() <= LIMIT => {
                                let store = buffer.get_backing_store();
                                let mut metadata = metadata;
                                metadata.body = store.iter().map(|byte| byte.get()).collect();
                                Ok(Some(metadata))
                            }
                            Some(_) => Err(WorkerError::new(
                                "QuotaExceededError",
                                "service response body exceeds limit",
                            )
                            .with_script_failure(crate::worker_wire::ScriptFailureKind::Capacity)),
                            None => Err(fetch_failure("service response body is invalid")),
                        });
                    }
                }
            } else if let Some(response) = response {
                let promise = v8::Local::new(scope, &response);
                match promise.state() {
                    v8::PromiseState::Pending => {}
                    v8::PromiseState::Rejected => {
                        result = Some(Err(fetch_failure("respondWith rejected")))
                    }
                    v8::PromiseState::Fulfilled => {
                        let value = promise.result(scope);
                        let extraction = {
                            v8::tc_scope!(let tc,scope);
                            extract_response(tc, state, value, &request)
                        };
                        match extraction {
                            Ok(body) => {
                                let mut body = Some(body);
                                if let Some(pending) =
                                    state.service_events.borrow_mut().pending.get_mut(&id)
                                    && !pending.response_done
                                {
                                    pending.body = body.take();
                                }
                                if let Some((_, _, Some(lease))) = body {
                                    let reason = dom_exception_value(scope, "AbortError")?;
                                    cleanup_attempt!(scope, |tc| lease.cancel(tc, reason));
                                }
                            }
                            Err(_) => {
                                result =
                                    Some(Err(failure("respondWith returned an unusable Response")))
                            }
                        }
                    }
                }
            }
            if let Some(result) = result {
                let handled = {
                    let mut events = state.service_events.borrow_mut();
                    let Some(pending) = events.pending.get_mut(&id) else {
                        continue;
                    };
                    if pending.response_done {
                        continue;
                    }
                    pending.response_done = true;
                    pending.handled.clone()
                };
                if let Some(handled) = handled {
                    let handled = v8::Local::new(scope, &handled);
                    match &result {
                        Ok(_) => {
                            let undefined = v8::undefined(scope);
                            handled.resolve(scope, undefined.into());
                        }
                        Err(error) => {
                            let message = v8::String::new(scope, &error.message).unwrap();
                            let exception = v8::Exception::type_error(scope, message);
                            handled.reject(scope, exception);
                        }
                    }
                }
                match result {
                    Ok(Some(response)) => output.push(AgentEvent::FetchResponse {
                        id: task,
                        response: Ok(response),
                    }),
                    Ok(None) => {}
                    Err(error) => output.push(AgentEvent::FetchResponse {
                        id: task,
                        response: Err(error),
                    }),
                }
            }
        }
        let complete = {
            let mut events = state.service_events.borrow_mut();
            let Some(pending) = events.pending.get_mut(&id) else {
                continue;
            };
            let mut rejected = false;
            pending
                .wait
                .retain(|promise| match v8::Local::new(scope, promise).state() {
                    v8::PromiseState::Pending => true,
                    v8::PromiseState::Fulfilled => false,
                    v8::PromiseState::Rejected => {
                        rejected = true;
                        false
                    }
                });
            pending.rejected |= rejected;
            !pending.dispatching
                && pending.pending_count == 0
                && (pending.request.is_none() || pending.response_done)
        };
        if complete {
            // Allocate before taking custody out of the event table.
            let reason = dom_exception_value(scope, "AbortError")?;
            let pending = state
                .service_events
                .borrow_mut()
                .pending
                .remove(&id)
                .unwrap();
            let cleanup_failed = pending.preload_signal.is_some_and(|signal| {
                cleanup_attempt!(scope, |tc| abort_signal_with_reason(
                    tc, state, signal, reason
                ))
            });
            output.push(AgentEvent::LifecycleFinished {
                id: task,
                result: if pending.rejected {
                    Err(WorkerError::new(
                        "AbortError",
                        "service event waitUntil rejected",
                    ))
                } else if cleanup_failed {
                    Err(failure("service event preload cleanup failed"))
                } else {
                    Ok(())
                },
            });
        }
    }
    Ok(())
}
pub(super) fn cancel_body(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    id: u64,
    reason: v8::Local<v8::Value>,
) -> Result<(), V8BindingError> {
    let reason = v8::Local::new(scope, reason);
    let (cancel, controller, host, consumer, upstream) = {
        let mut record = platform_object_mut(state, id)?;
        let PlatformObjectRecord::ReadableStream(stream) = &mut *record else {
            return Err(V8BindingError::Allocation("service response stream"));
        };
        stream.queue.clear();
        stream.disturbed = true;
        (
            stream.cancel.clone(),
            stream.controller_id,
            stream.host_stream_id,
            stream.consumer.take(),
            stream.tee_source,
        )
    };
    if let Some(consumer) = consumer {
        stream_graph::local(scope, &consumer.resolver).reject(scope, reason);
    }
    // Cancellation discards the queue and closes even when byte-stream close
    // was already requested. The author-facing controller close rejects that
    // state, so use the same internal sequence as ReadableStream.cancel.
    let mut failed = cleanup_attempt!(scope, |tc| readable_close_default(tc, state, controller));
    failed |= cleanup_attempt!(scope, |tc| byte_streams::cancel_pending(tc, state, id));
    failed |= cleanup_attempt!(scope, |tc| readable_streams::settle_closed(tc, state, id));
    if let Some((source, index)) = upstream {
        failed |= cleanup_attempt!(scope, |tc| stream_tee::cancel(
            tc, state, source, index, reason
        ));
    } else {
        failed |= cleanup_attempt!(scope, |tc| transform_streams::cancel_readable(
            tc, state, id, reason
        ));
        if let Some(stream_id) = host {
            // Admission failure transfers native custody to abandon_http_stream.
            failed |= submit_platform_operation(
                state,
                PlatformHostRequest::HttpAbort { stream_id },
                PendingPlatformOperation::WebSocketCommand,
            )
            .is_err();
        }
        failed |= cleanup_attempt!(scope, |tc| readable_streams::cancel(
            tc, state, id, cancel, reason
        ));
    }
    if failed {
        Err(V8BindingError::Allocation("service body cleanup failed"))
    } else {
        Ok(())
    }
}

pub fn cancel(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    task: u64,
) -> Result<Vec<AgentEvent>, V8BindingError> {
    let id = state
        .service_events
        .borrow()
        .pending
        .iter()
        .find_map(|(id, pending)| (pending.task == task).then_some(*id));
    let Some(id) = id else { return Ok(Vec::new()) };
    let reason = dom_exception_value(scope, "AbortError")?;
    let mut pending = state
        .service_events
        .borrow_mut()
        .pending
        .remove(&id)
        .unwrap();
    if let Some(signal) = pending.preload_signal.take() {
        cleanup_attempt!(scope, |tc| abort_signal_with_reason(
            tc, state, signal, reason
        ));
    }
    if let Some(mut stream) = pending.stream.take() {
        cleanup_attempt!(scope, |tc| stream.cancel(tc, state, reason));
    }
    if let Some((_, _, Some(stream))) = pending.body {
        cleanup_attempt!(scope, |tc| stream.cancel(tc, reason));
    }
    if let Some(handled) = pending.handled {
        v8::Local::new(scope, &handled).reject(scope, reason);
    }
    if pending.request.is_some() {
        cleanup_attempt!(scope, |tc| (|| {
            let event = v8::Local::new(tc, &pending.event);
            let request = attribute(tc, state, event, "request")?;
            if let Ok(request) = v8::Local::<v8::Object>::try_from(request)
                && let Ok(id) = stateful_id(tc, request)
            {
                let signal = match &*platform_object(state, id)? {
                    PlatformObjectRecord::Request(request) => Some(request.signal_id),
                    _ => None,
                };
                if let Some(signal) = signal {
                    abort_signal_with_reason(tc, state, signal, reason)?;
                }
            }
            Ok(())
        })());
    }
    let failure = WorkerError::new("AbortError", "service event was cancelled")
        .with_script_failure(crate::worker_wire::ScriptFailureKind::Cancelled);
    let mut output = Vec::new();
    if pending.request.is_some() && !pending.response_done {
        output.push(if pending.stream_started {
            AgentEvent::FetchResponseEnd {
                id: task,
                result: Err(failure.clone()),
            }
        } else {
            AgentEvent::FetchResponse {
                id: task,
                response: Err(failure.clone()),
            }
        });
    }
    output.push(AgentEvent::LifecycleFinished {
        id: task,
        result: Err(failure),
    });
    Ok(output)
}

pub fn pull(state: &Rc<V8BindingState>, task: u64) -> Result<(), V8BindingError> {
    let mut events = state.service_events.borrow_mut();
    let pending = events
        .pending
        .values_mut()
        .find(|pending| pending.task == task)
        .ok_or(V8BindingError::Allocation("unknown service body demand"))?;
    if !pending
        .request
        .as_ref()
        .is_some_and(|request| request.streaming)
        || !pending.stream_started
        || pending.response_done
    {
        return Err(V8BindingError::Allocation(
            "service body demand outside live response",
        ));
    }
    if pending.credits >= 2 {
        return Err(V8BindingError::Allocation("service body demand capacity"));
    }
    pending.credits += 1;
    Ok(())
}
fn extract_stream(
    scope: &mut v8::PinScope,
    _state: &Rc<V8BindingState>,
    value: v8::Local<v8::Value>,
    request: &AgentFetchRequest,
) -> Result<(AgentFetchResponse, Option<service_body::Reader>), V8BindingError> {
    let object = v8::Local::<v8::Object>::try_from(value)
        .map_err(|_| V8BindingError::Allocation("streaming Response"))?;
    if object.is_proxy() || object.internal_field_count() == 0 {
        return Err(V8BindingError::Allocation("streaming Response brand"));
    }
    let owner = binding_state_for_receiver(scope, object)
        .ok_or(V8BindingError::Allocation("Response native owner"))?;
    let state = &owner;
    let id = stateful_id(scope, object)?;
    if !state
        .platform_wrappers
        .borrow()
        .get(&id)
        .and_then(|weak| weak.to_local(scope))
        .is_some_and(|wrapper| wrapper.strict_equals(value))
    {
        return Err(V8BindingError::Allocation("Response ownership"));
    }
    let (metadata, stream, foreign) = {
        let record = platform_object(state, id)?;
        let PlatformObjectRecord::Response(response) = &*record else {
            return Err(V8BindingError::Allocation("streaming Response record"));
        };
        if response.body_used
            || response.response_type == "error"
            || (response.response_type == "opaque" && request.mode != "no-cors")
            || (response.response_type == "opaqueredirect" && request.redirect != "manual")
        {
            return Err(V8BindingError::Allocation("unusable streaming Response"));
        }
        let headers = match &*platform_object(state, response.headers_id)? {
            PlatformObjectRecord::Headers(headers) => headers.entries.clone(),
            _ => return Err(V8BindingError::Allocation("Response headers")),
        };
        {
            let internal = response.internal_response.as_ref();
            if response.response_type == "opaque"
                && internal
                    .is_some_and(|internal| internal.status == 206 && internal.range_requested)
                && !request
                    .headers
                    .iter()
                    .any(|(name, _)| name.eq_ignore_ascii_case("range"))
            {
                return Err(V8BindingError::Allocation(
                    "opaque ranged response requires a range request",
                ));
            }
            (
                AgentFetchResponse {
                    timing: response.timing.clone(),
                    redirected: internal
                        .map_or(response.redirected, |internal| internal.redirected),
                    range_requested: internal.is_some_and(|internal| internal.range_requested),
                    status: internal.map_or(response.status, |internal| internal.status),
                    status_text: response.status_text.clone(),
                    headers: internal.map_or(headers, |internal| internal.headers.clone()),
                    body: Vec::new(),
                    response_url: internal
                        .map_or_else(|| response.url.clone(), |internal| internal.url.clone()),
                    response_type: response.response_type.into(),
                },
                response.body_stream_id,
                response.body_owner.clone(),
            )
        }
    };
    let reader = stream
        .map(|stream| service_body::Reader::acquire(scope, &body_owner(state, &foreign)?, stream))
        .transpose()?;
    if reader.is_some()
        && let PlatformObjectRecord::Response(response) = &mut *platform_object_mut(state, id)?
    {
        response.body_used = true;
    }
    Ok((metadata, reader))
}
fn finish_stream(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    id: u64,
    result: Result<(), WorkerError>,
    output: &mut Vec<AgentEvent>,
) -> Result<(), V8BindingError> {
    // Fail allocations before marking response_done; cleanup cannot veto the
    // original response outcome once native terminal custody is committed.
    let reason = dom_exception_value(scope, "NetworkError")?;
    let (task, started, handled, mut stream) = {
        let mut events = state.service_events.borrow_mut();
        let Some(pending) = events.pending.get_mut(&id) else {
            return Ok(());
        };
        if pending.response_done {
            return Ok(());
        }
        pending.response_done = true;
        pending.credits = 0;
        (
            pending.task,
            pending.stream_started,
            pending.handled.clone(),
            pending.stream.take(),
        )
    };
    if result.is_err()
        && let Some(stream) = stream.as_mut()
    {
        cleanup_attempt!(scope, |tc| stream.cancel(tc, state, reason));
    }
    if let Some(handled) = handled {
        let resolver = v8::Local::new(scope, &handled);
        match &result {
            Ok(()) => {
                let value = v8::undefined(scope);
                resolver.resolve(scope, value.into());
            }
            Err(_) => {
                resolver.reject(scope, reason);
            }
        }
    }
    output.push(if started {
        AgentEvent::FetchResponseEnd { id: task, result }
    } else {
        AgentEvent::FetchResponse {
            id: task,
            response: Err(result
                .err()
                .unwrap_or_else(|| failure("response ended before headers"))),
        }
    });
    Ok(())
}

fn poll_streaming(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    id: u64,
    output: &mut Vec<AgentEvent>,
) -> Result<(), V8BindingError> {
    let (task, started, response, request, credits) = {
        let events = state.service_events.borrow();
        let Some(pending) = events.pending.get(&id) else {
            return Ok(());
        };
        (
            pending.task,
            pending.stream_started,
            pending.response.clone(),
            pending.request.clone().unwrap(),
            pending.credits,
        )
    };
    if !started {
        let Some(response) = response else {
            return Ok(());
        };
        let response = v8::Local::new(scope, &response);
        match response.state() {
            v8::PromiseState::Pending => return Ok(()),
            v8::PromiseState::Rejected => {
                return finish_stream(
                    scope,
                    state,
                    id,
                    Err(fetch_failure("respondWith rejected")),
                    output,
                );
            }
            v8::PromiseState::Fulfilled => {}
        }
        let value = response.result(scope);
        let extracted = {
            v8::tc_scope!(let tc,scope);
            extract_stream(tc, state, value, &request)
        };
        let (metadata, reader) = match extracted {
            Ok(value) => value,
            Err(_) => {
                return finish_stream(
                    scope,
                    state,
                    id,
                    Err(failure("unusable streaming Response")),
                    output,
                );
            }
        };
        let empty = reader.is_none();
        {
            let mut events = state.service_events.borrow_mut();
            let pending = events.pending.get_mut(&id).unwrap();
            pending.stream_started = true;
            pending.stream = reader;
        }
        output.push(AgentEvent::FetchResponseStart {
            id: task,
            response: metadata,
        });
        if empty {
            finish_stream(scope, state, id, Ok(()), output)?;
        }
        return Ok(());
    }
    if credits == 0 {
        return Ok(());
    }
    let mut reader = state
        .service_events
        .borrow_mut()
        .pending
        .get_mut(&id)
        .and_then(|pending| pending.stream.take())
        .ok_or(V8BindingError::Allocation("live streaming body reader"))?;
    let result = {
        v8::tc_scope!(let tc,scope);
        reader.poll(tc, state)
    };
    // Pull can invoke author code which cancels this event. Its terminal
    // outcome then belongs to cancellation; discard the late chunk and release
    // the reader taken out of the event table before the callback.
    let still_live = state
        .service_events
        .borrow()
        .pending
        .get(&id)
        .is_some_and(|pending| pending.task == task && !pending.response_done);
    if !still_live {
        let reason = dom_exception_value(scope, "AbortError")?;
        cleanup_attempt!(scope, |tc| reader.cancel(tc, state, reason));
        return Ok(());
    }
    state
        .service_events
        .borrow_mut()
        .pending
        .get_mut(&id)
        .unwrap()
        .stream = Some(reader);
    match result {
        Ok(service_body::Poll::Pending) => {}
        Ok(service_body::Poll::Chunk(bytes)) => {
            state
                .service_events
                .borrow_mut()
                .pending
                .get_mut(&id)
                .unwrap()
                .credits -= 1;
            output.push(AgentEvent::FetchResponseChunk { id: task, bytes });
        }
        Ok(service_body::Poll::Done) => finish_stream(scope, state, id, Ok(()), output)?,
        Err(_) => finish_stream(
            scope,
            state,
            id,
            Err(failure("service response stream failed")),
            output,
        )?,
    }
    Ok(())
}

#[cfg(test)]
mod terminal_cleanup_tests {
    use super::*;
    fn request(task: u64) -> AgentFetchRequest {
        AgentFetchRequest {
            keepalive: false,
            privileged_range: false,
            cache: Default::default(),
            navigation: Default::default(),
            integrity: String::new(),
            navigation_preload: false,
            streaming: true,
            id: task,
            client_id: String::new(),
            url: "https://example.test/".into(),
            method: "GET".into(),
            headers: Vec::new(),
            body: Vec::new(),
            destination: String::new(),
            mode: "cors".into(),
            credentials: "same-origin".into(),
            redirect: "follow".into(),
        }
    }
    #[test]
    fn opaque_range_replay_checks_internal_provenance_before_consuming_body() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(
            scope,
            context,
            Rc::new(RefCell::new(oryn_dom::DomRuntime::new())),
            RealmId(92),
            1,
        )
        .unwrap();
        let stream = new_byte_stream_record(scope, &state, b"partial bytes".to_vec()).unwrap();
        let headers_id = headers_from_entries(&state, Vec::new()).unwrap();
        let id = register_platform_object(
            &state,
            PlatformObjectRecord::Response(ResponseRecord {
                timing: None,
                internal_response: Some(crate::platform::InternalResponseMetadata {
                    status: 206,
                    headers: vec![("content-range".into(), "bytes 0-12/100".into())],
                    url: "https://foreign.test/data".into(),
                    redirected: false,
                    range_requested: true,
                }),
                body_owner: None,
                response_type: "opaque",
                status_text: String::new(),
                redirected: false,
                status: 0,
                url: String::new(),
                headers_id,
                body_stream_id: Some(stream),
                bytes: Vec::new(),
                body_used: false,
                signal_id: None,
            }),
        );
        let value = new_platform_object(scope, &state, "Response", id).unwrap();
        let mut request = request(1);
        request.mode = "no-cors".into();
        assert!(extract_stream(scope, &state, value.into(), &request).is_err());
        assert!(extract_response(scope, &state, value.into(), &request).is_err());
        assert_eq!(
            response_stream_state(&state, Some(stream)).unwrap(),
            (false, false)
        );
        request.headers.push(("range".into(), "bytes=0-12".into()));
        let (metadata, reader) = extract_stream(scope, &state, value.into(), &request).unwrap();
        assert_eq!(metadata.status, 206);
        assert!(metadata.range_requested);
        assert_eq!(metadata.response_url, "https://foreign.test/data");
        assert!(reader.is_some());
    }
    #[test]
    fn readable_partial_service_response_is_accepted_without_range_request() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(
            scope,
            context,
            Rc::new(RefCell::new(oryn_dom::DomRuntime::new())),
            RealmId(94),
            1,
        )
        .unwrap();
        for response_type in ["basic", "cors", "default"] {
            let stream =
                new_byte_stream_record(scope, &state, b"self.scriptExecuted=true".to_vec())
                    .unwrap();
            let headers_id = headers_from_entries(
                &state,
                vec![vec!["content-type".into(), "text/javascript".into()]],
            )
            .unwrap();
            let id = register_platform_object(
                &state,
                PlatformObjectRecord::Response(ResponseRecord {
                    timing: None,
                    internal_response: None,
                    body_owner: None,
                    response_type,
                    status_text: String::new(),
                    redirected: false,
                    status: 206,
                    url: "https://example.test/partial".into(),
                    headers_id,
                    body_stream_id: Some(stream),
                    bytes: Vec::new(),
                    body_used: false,
                    signal_id: None,
                }),
            );
            let value = new_platform_object(scope, &state, "Response", id).unwrap();
            let mut request = request(55);
            request.destination = "script".into();
            let (metadata, reader) = extract_stream(scope, &state, value.into(), &request).unwrap();
            assert_eq!(metadata.status, 206);
            assert_eq!(metadata.response_type, response_type);
            assert!(reader.is_some());
        }
    }
    #[test]
    fn native_range_request_clone_and_successful_header_mutations_follow_guards() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(
            scope,
            context,
            Rc::new(RefCell::new(oryn_dom::DomRuntime::new())),
            RealmId(93),
            1,
        )
        .unwrap();
        let source=v8::String::new(scope,r#"
          globalThis.checked=false;addEventListener('fetch',e=>{
            const r=e.request,check=(v)=>{if(!v)throw Error('native range guard')};
            for(const c of [r.clone(),new Request(r),new Request(r,{}),new Request(r,{unknown:true}),new Request(r,{mode:'cors'})])check(c.headers.get('range')==='bytes=0-');
            for(const init of [{mode:'no-cors'},{cache:'no-cache'},{headers:{}},{headers:{Range:'foo'}}])check(new Request(r,init).headers.get('range')===null);
            for(const [method,name,value] of [['delete','missing'],['append','foo','bar'],['set','foo','bar'],['append','range','foo'],['set','range','foo']]){const h=new Request(r).headers;h[method](name,value);check(h.get('range')==='bytes=0-')}
            for(const [method,name,value] of [['append','accept','text/plain'],['set','accept','text/plain'],['delete','accept'],['delete','range']]){const h=new Request(r).headers;h[method](name,value);check(h.get('range')===null)}
            checked=true;
          });
        "#).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        let mut request = request(40);
        request.mode = "no-cors".into();
        request.destination = "audio".into();
        request.privileged_range = true;
        request.headers = vec![
            ("range".into(), "bytes=0-".into()),
            ("accept".into(), "*/*".into()),
        ];
        begin(scope, &state, AgentCommand::Fetch(request)).unwrap();
        let source = v8::String::new(scope, "checked").unwrap();
        assert!(
            v8::Script::compile(scope, source, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
    }
    fn cancel_from_pull(
        scope: &mut v8::PinScope,
        _: v8::FunctionCallbackArguments,
        _: v8::ReturnValue,
    ) {
        let state = scope
            .get_current_context()
            .get_slot::<V8BindingState>()
            .unwrap();
        let events = cancel(scope, &state, 77).unwrap();
        state.service_events.borrow_mut().ready.extend(events);
    }
    #[test]
    fn native_navigation_flags_clone_and_nonempty_request_init_reset() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let source=v8::String::new(scope,r#"
            globalThis.results=[];addEventListener('fetch',event=>{
                const r=event.request,clone=r.clone(),empty=new Request(r,{}),unknown=new Request(r,{unknown:true});
                const calls=[];const init=Object.create({get cache(){calls.push('cache');return 'no-store'}});
                Object.defineProperty(init,'mode',{get(){calls.push('mode');return undefined}});
                const reset=new Request(r,init);const ignored=new Request(r,{cache:undefined});
                results.push([r.isReloadNavigation,r.isHistoryNavigation,r.mode,r.destination,
                    clone.isReloadNavigation,clone.isHistoryNavigation,empty.mode,unknown.mode,ignored.mode,
                    reset.isReloadNavigation,reset.isHistoryNavigation,reset.mode,reset.destination,calls.join(',')]);
            });
        "#).unwrap();
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap();
        for (index, navigation) in [
            crate::platform::NavigationRequestFlags {
                reload: true,
                history: false,
            },
            crate::platform::NavigationRequestFlags {
                reload: false,
                history: true,
            },
        ]
        .into_iter()
        .enumerate()
        {
            let mut request = request(100 + index as u64);
            request.navigation = navigation;
            request.mode = "navigate".into();
            request.destination = "iframe".into();
            begin(scope, &state, AgentCommand::Fetch(request)).unwrap();
        }
        let source = v8::String::new(scope, "JSON.stringify(results)").unwrap();
        let result = v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
            .to_string(scope)
            .unwrap()
            .to_rust_string_lossy(scope);
        let rows: serde_json::Value = serde_json::from_str(&result).unwrap();
        for (index, row) in rows.as_array().unwrap().iter().enumerate() {
            assert_eq!(
                row,
                &serde_json::json!([
                    index == 0,
                    index == 1,
                    "navigate",
                    "iframe",
                    index == 0,
                    index == 1,
                    "navigate",
                    "navigate",
                    "navigate",
                    false,
                    false,
                    "same-origin",
                    "",
                    "cache,mode"
                ])
            );
        }
    }
    #[test]
    fn request_cache_modes_preserve_clone_and_validate_inherited_mode() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        state.commit_location("https://example.test/").unwrap();
        let code=v8::String::new(scope,r#"for(const cache of ['default','no-store','reload','no-cache','force-cache','only-if-cached']){
            const request=new Request('https://example.test/',{cache,mode:'same-origin'});
            if(request.cache!==cache||request.clone().cache!==cache||new Request(request,{}).cache!==cache)throw Error('cache clone');
        }const cached=new Request('https://example.test/',{cache:'only-if-cached',mode:'same-origin'});
        let rejected=false;try{new Request(cached,{mode:'cors'})}catch(e){rejected=true}if(!rejected)throw Error('inherited cache mode must validate');"#).unwrap();
        v8::Script::compile(scope, code, None)
            .unwrap()
            .run(scope)
            .unwrap();
    }

    #[test]
    fn reentrant_pull_cancellation_discards_late_result_and_releases_reader() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let callback = v8::Function::new(scope, cancel_from_pull).unwrap();
        let key = v8::String::new(scope, "cancelFromPull").unwrap();
        context
            .global(scope)
            .set(scope, key.into(), callback.into())
            .unwrap();
        let code = v8::String::new(
            scope,
            "new ReadableStream({pull:cancelFromPull},{highWaterMark:0})",
        )
        .unwrap();
        let body = v8::Script::compile(scope, code, None)
            .unwrap()
            .run(scope)
            .unwrap();
        scope.perform_microtask_checkpoint();
        let object = v8::Local::<v8::Object>::try_from(body).unwrap();
        let stream = stateful_id(scope, object).unwrap();
        begin(scope, &state, AgentCommand::Fetch(request(77))).unwrap();
        let reader = service_body::Reader::acquire(scope, &state, stream).unwrap();
        let id = {
            let mut events = state.service_events.borrow_mut();
            let (id, pending) = events.pending.iter_mut().next().unwrap();
            pending.stream = Some(reader);
            pending.stream_started = true;
            pending.responded = true;
            pending.credits = 1;
            *id
        };
        let mut output = Vec::new();
        poll_streaming(scope, &state, id, &mut output).unwrap();
        assert!(output.is_empty());
        assert!(state.service_events.borrow().pending.is_empty());
        {
            let record = platform_object(&state, stream).unwrap();
            let PlatformObjectRecord::ReadableStream(record) = &*record else {
                panic!()
            };
            assert!(!record.locked);
            assert!(record.disturbed);
        }
        let events = poll(scope, &state).unwrap();
        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            AgentEvent::FetchResponseEnd {
                id: 77,
                result: Err(_)
            }
        ));
        assert!(matches!(
            &events[1],
            AgentEvent::LifecycleFinished {
                id: 77,
                result: Err(_)
            }
        ));
        assert!(poll(scope, &state).unwrap().is_empty());
    }
    #[test]
    fn reentrant_buffered_consumption_cancels_unclaimed_body_lease() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        let callback = v8::Function::new(scope, cancel_from_pull).unwrap();
        let key = v8::String::new(scope, "cancelFromPull").unwrap();
        context
            .global(scope)
            .set(scope, key.into(), callback.into())
            .unwrap();
        let code=v8::String::new(scope,"globalThis.cancelCount=0;globalThis.body=new ReadableStream({pull:cancelFromPull,cancel(){cancelCount++}},{highWaterMark:0});new Response(body)").unwrap();
        let response = v8::Script::compile(scope, code, None)
            .unwrap()
            .run(scope)
            .unwrap();
        scope.perform_microtask_checkpoint();
        let mut request = request(77);
        request.streaming = false;
        begin(scope, &state, AgentCommand::Fetch(request)).unwrap();
        let response = promise(scope, response).unwrap();
        {
            let mut events = state.service_events.borrow_mut();
            let pending = events.pending.values_mut().next().unwrap();
            pending.responded = true;
            pending.response = Some(response);
        }
        let output = poll(scope, &state).unwrap();
        assert!(output.is_empty());
        assert!(state.service_events.borrow().pending.is_empty());
        let code = v8::String::new(scope, "cancelCount===1").unwrap();
        assert!(
            v8::Script::compile(scope, code, None)
                .unwrap()
                .run(scope)
                .unwrap()
                .is_true()
        );
        let events = poll(scope, &state).unwrap();
        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            AgentEvent::FetchResponse {
                id: 77,
                response: Err(_)
            }
        ));
        assert!(matches!(
            &events[1],
            AgentEvent::LifecycleFinished {
                id: 77,
                result: Err(_)
            }
        ));
        assert!(poll(scope, &state).unwrap().is_empty());
    }
    #[test]
    fn cancellation_attempts_reader_after_preload_failure_and_publishes_once() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        begin(scope, &state, AgentCommand::Fetch(request(71))).unwrap();
        let stream = new_byte_stream_record(scope, &state, vec![1, 2]).unwrap();
        let reader = service_body::Reader::acquire(scope, &state, stream).unwrap();
        // Fault injection: the preload signal is gone and closing the stream's
        // controller fails. Releasing its reader and publishing must still run.
        if let PlatformObjectRecord::ReadableStream(record) =
            &mut *platform_object_mut(&state, stream).unwrap()
        {
            record.controller_id = u64::MAX;
        }
        let handled = {
            let mut events = state.service_events.borrow_mut();
            let pending = events.pending.values_mut().next().unwrap();
            pending.preload_signal = Some(u64::MAX);
            pending.stream = Some(reader);
            pending.stream_started = true;
            pending.responded = true;
            pending.handled.clone().unwrap()
        };
        let events = cancel(scope, &state, 71).unwrap();
        assert_eq!(events.len(), 2);
        assert!(
            matches!(&events[0],AgentEvent::FetchResponseEnd{id:71,result:Err(error)} if error.name=="AbortError")
        );
        assert!(matches!(
            &events[1],
            AgentEvent::LifecycleFinished {
                id: 71,
                result: Err(_)
            }
        ));
        let handled = v8::Local::new(scope, &handled);
        assert_eq!(
            handled.get_promise(scope).state(),
            v8::PromiseState::Rejected
        );
        {
            let record = platform_object(&state, stream).unwrap();
            let PlatformObjectRecord::ReadableStream(record) = &*record else {
                panic!()
            };
            assert!(!record.locked);
            assert!(record.disturbed);
        }
        assert!(cancel(scope, &state, 71).unwrap().is_empty());
        assert!(state.service_events.borrow().pending.is_empty());
    }
    #[test]
    fn failed_stream_cleanup_cannot_suppress_response_end_or_handled() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        begin(scope, &state, AgentCommand::Fetch(request(72))).unwrap();
        let stream = new_byte_stream_record(scope, &state, vec![3]).unwrap();
        let reader = service_body::Reader::acquire(scope, &state, stream).unwrap();
        if let PlatformObjectRecord::ReadableStream(record) =
            &mut *platform_object_mut(&state, stream).unwrap()
        {
            record.controller_id = u64::MAX;
        }
        let (id, handled) = {
            let mut events = state.service_events.borrow_mut();
            let (id, pending) = events.pending.iter_mut().next().unwrap();
            pending.stream = Some(reader);
            pending.stream_started = true;
            pending.responded = true;
            (*id, pending.handled.clone().unwrap())
        };
        let mut events = Vec::new();
        finish_stream(
            scope,
            &state,
            id,
            Err(failure("original failure")),
            &mut events,
        )
        .unwrap();
        assert!(
            matches!(&events[0],AgentEvent::FetchResponseEnd{id:72,result:Err(error)} if error.message=="original failure")
        );
        let handled = v8::Local::new(scope, &handled);
        assert_eq!(
            handled.get_promise(scope).state(),
            v8::PromiseState::Rejected
        );
        finish_stream(scope, &state, id, Err(failure("duplicate")), &mut events).unwrap();
        assert_eq!(events.len(), 1);
        let lifecycle = poll(scope, &state).unwrap();
        assert_eq!(lifecycle.len(), 1);
        assert!(matches!(
            &lifecycle[0],
            AgentEvent::LifecycleFinished { id: 72, .. }
        ));
    }
    #[test]
    fn lifecycle_preload_cleanup_failure_still_publishes_terminal_error() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        begin(
            scope,
            &state,
            AgentCommand::Lifecycle {
                id: 73,
                event: LifecycleEvent::Activate,
                payload: None,
            },
        )
        .unwrap();
        state
            .service_events
            .borrow_mut()
            .pending
            .values_mut()
            .next()
            .unwrap()
            .preload_signal = Some(u64::MAX);
        let events = poll(scope, &state).unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            AgentEvent::LifecycleFinished {
                id: 73,
                result: Err(_)
            }
        ));
        assert!(poll(scope, &state).unwrap().is_empty());
    }
    #[test]
    fn later_poll_error_retains_previously_committed_lifecycle_receipt() {
        super::super::tests::initialize_v8();
        let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let state = install_generated_bindings(scope, context, dom, RealmId(0), 1).unwrap();
        begin(
            scope,
            &state,
            AgentCommand::Lifecycle {
                id: 74,
                event: LifecycleEvent::Activate,
                payload: None,
            },
        )
        .unwrap();
        begin(scope, &state, AgentCommand::Fetch(request(75))).unwrap();
        let broken = {
            let mut events = state.service_events.borrow_mut();
            let (id, pending) = events
                .pending
                .iter_mut()
                .find(|(_, p)| p.task == 75)
                .unwrap();
            pending.responded = true;
            pending.stream_started = true;
            pending.credits = 1;
            *id
        };
        assert!(poll(scope, &state).is_err());
        assert!(
            state
                .service_events
                .borrow()
                .ready
                .iter()
                .any(|event| matches!(event, AgentEvent::LifecycleFinished { id: 74, .. }))
        );
        state.service_events.borrow_mut().pending.remove(&broken);
        let delivered = poll(scope, &state).unwrap();
        assert_eq!(delivered.len(), 1);
        assert!(matches!(
            &delivered[0],
            AgentEvent::LifecycleFinished { id: 74, .. }
        ));
        assert!(poll(scope, &state).unwrap().is_empty());
    }
}
