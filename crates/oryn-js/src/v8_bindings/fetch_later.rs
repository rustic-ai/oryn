//! Deferred Fetch admission and Document-owned activation custody.
use super::*;
use crate::fetch_later::{
    DeferredFetchBackend, DeferredFetchError, DeferredFetchState, DeferredFetchToken,
};
use std::time::{Duration, Instant};
#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Waiting,
    Activate,
    Cancel,
    Done,
}
const MAX_RECORDS: usize = 1024;
struct Record {
    request_root: Option<v8::Global<v8::Object>>,
    signal_root: Option<v8::Global<v8::Object>>,
    object: v8::Weak<v8::Object>,
    token: DeferredFetchToken,
    backend: Rc<dyn DeferredFetchBackend>,
    signal: u64,
    deadline: Option<Instant>,
    action: Action,
    activated: bool,
}
#[derive(Default)]
pub(super) struct Registry {
    next: u64,
    token_sequence: u64,
    prototype: Option<v8::Weak<v8::Object>>,
    records: BTreeMap<u64, Record>,
}
fn error(name: &str) -> V8BindingError {
    V8BindingError::Unsupported {
        interface: "fetchLater".into(),
        member: name.into(),
    }
}
fn key<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Private> {
    let name = v8::String::new(scope, "oryn.fetch-later.result").unwrap();
    v8::Private::for_api(scope, Some(name))
}
fn activated_key<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Private> {
    let name = v8::String::new(scope, "oryn.fetch-later.activated").unwrap();
    v8::Private::for_api(scope, Some(name))
}
fn refresh_status(state: &V8BindingState, id: u64) {
    let capability = {
        let registry = state.deferred_fetch.borrow();
        registry
            .records
            .get(&id)
            .filter(|record| record.action != Action::Done)
            .map(|record| (record.backend.clone(), record.token.clone()))
    };
    let Some((backend, token)) = capability else {
        return;
    };
    let result = backend.state(&token);
    if let Some(record) = state.deferred_fetch.borrow_mut().records.get_mut(&id) {
        match result {
            Ok(DeferredFetchState::Activated) => {
                record.activated = true;
                record.action = Action::Done;
            }
            Ok(DeferredFetchState::Canceled)
            | Err(DeferredFetchError::Type(_) | DeferredFetchError::Quota(_)) => {
                record.action = Action::Done;
            }
            Ok(DeferredFetchState::Pending) | Err(DeferredFetchError::Transport(_)) => {}
        }
        if record.action == Action::Done {
            record.deadline = None;
            record.request_root = None;
            record.signal_root = None;
        }
    }
}
fn refresh_all(state: &V8BindingState) {
    let ids = state
        .deferred_fetch
        .borrow()
        .records
        .keys()
        .copied()
        .collect::<Vec<_>>();
    for id in ids {
        refresh_status(state, id)
    }
}
fn finalize(scope: &mut v8::PinScope, state: &V8BindingState) -> Result<(), V8BindingError> {
    let ids = state
        .deferred_fetch
        .borrow()
        .records
        .iter()
        .filter_map(|(id, record)| (record.action == Action::Done).then_some(*id))
        .collect::<Vec<_>>();
    for id in ids {
        let terminal = {
            let registry = state.deferred_fetch.borrow();
            registry
                .records
                .get(&id)
                .map(|record| (record.object.to_local(scope), record.activated))
        };
        if let Some((Some(object), activated)) = terminal {
            let key = activated_key(scope);
            let value = v8::Boolean::new(scope, activated);
            if object.set_private(scope, key, value.into()) != Some(true) {
                return Err(V8BindingError::JavaScriptException);
            }
        }
        acknowledge(state, id);
    }
    Ok(())
}
fn acknowledge(state: &V8BindingState, id: u64) {
    let capability = {
        let registry = state.deferred_fetch.borrow();
        registry
            .records
            .get(&id)
            .map(|record| (record.backend.clone(), record.token.clone()))
    };
    let Some((backend, token)) = capability else {
        return;
    };
    if matches!(
        backend.acknowledge(&token),
        Err(DeferredFetchError::Transport(_))
    ) {
        if let Some(record) = state.deferred_fetch.borrow_mut().records.get_mut(&id) {
            record.deadline = Instant::now().checked_add(Duration::from_millis(25));
        }
    } else {
        state.deferred_fetch.borrow_mut().records.remove(&id);
    }
}
fn backend_error(value: DeferredFetchError) -> V8BindingError {
    error(match value {
        DeferredFetchError::Quota(_) => "QuotaExceededError",
        DeferredFetchError::Type(_) => "TypeError",
        DeferredFetchError::Transport(_) => "InvalidStateError",
    })
}
fn illegal(scope: &mut v8::PinScope, _args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue) {
    throw_type_error(scope, "Illegal constructor")
}
fn activated(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    let result = (|| {
        let object = args.this();
        if object.is_proxy() {
            return Err(error("TypeError"));
        }
        let state = object
            .get_creation_context(scope)
            .and_then(|context| context.get_slot::<V8BindingState>())
            .ok_or_else(|| error("TypeError"))?;
        let brand = key(scope);
        let id = object
            .get_private(scope, brand)
            .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
            .map(|value| value.u64_value().0)
            .ok_or_else(|| error("TypeError"))?;
        let terminal = activated_key(scope);
        let value = object
            .get_private(scope, terminal)
            .ok_or(V8BindingError::JavaScriptException)?;
        if value.is_boolean() {
            return Ok(value.is_true());
        }
        refresh_status(&state, id);
        let (value, done) = {
            let registry = state.deferred_fetch.borrow();
            let record = registry
                .records
                .get(&id)
                .ok_or_else(|| error("TypeError"))?;
            if !record
                .object
                .to_local(scope)
                .is_some_and(|registered| registered.strict_equals(object.into()))
            {
                return Err(error("TypeError"));
            }
            (record.activated, record.action == Action::Done)
        };
        if done {
            let stored = v8::Boolean::new(scope, value);
            if object.set_private(scope, terminal, stored.into()) != Some(true) {
                return Err(V8BindingError::JavaScriptException);
            }
            acknowledge(&state, id);
        }
        Ok(value)
    })();
    match result {
        Ok(value) => rv.set_bool(value),
        Err(err) => throw_binding_error(scope, &err),
    }
}
fn schedule(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut rv: v8::ReturnValue,
) {
    v8::tc_scope!(let scope,scope);
    let result = (|| {
        if args.length() == 0 {
            return Err(error("TypeError"));
        }
        let state = scope
            .get_current_context()
            .get_slot::<V8BindingState>()
            .ok_or_else(|| error("InvalidStateError"))?;
        if state.execution_retired() || state.execution_suspended.get() {
            return Err(error("InvalidStateError"));
        }
        refresh_all(&state);
        finalize(scope, &state)?;
        {
            let registry = state.deferred_fetch.borrow();
            if registry.records.len() >= MAX_RECORDS {
                return Err(error("QuotaExceededError"));
            }
        }
        let generation = state.document_generation();
        let authority = state.document_authority_epoch();
        let request_id = create_request_record(scope, &state, args.get(0), args.get(1))?;
        let deadline =
            if let Some(value) = checked_dictionary_member(scope, args.get(1), "activateAfter")? {
                let ms = value
                    .number_value(scope)
                    .ok_or(V8BindingError::JavaScriptException)?;
                if !ms.is_finite() {
                    return Err(error("TypeError"));
                }
                if ms < 0.0 {
                    return Err(error("RangeError"));
                }
                Some(
                    Instant::now()
                        .checked_add(
                            Duration::try_from_secs_f64(ms / 1000.0)
                                .map_err(|_| error("RangeError"))?,
                        )
                        .ok_or_else(|| error("RangeError"))?,
                )
            } else {
                None
            };
        let (request, signal) = {
            let record = platform_object(&state, request_id)?;
            let PlatformObjectRecord::Request(request) = &*record else {
                return Err(error("TypeError"));
            };
            if !request.body_source_known {
                return Err(error("TypeError"));
            }
            let url = url::Url::parse(&request.url).map_err(|_| error("TypeError"))?;
            if !matches!(url.scheme(), "http" | "https")
                || !crate::realm::potentially_trustworthy_origin(&crate::origin::Origin::from_url(
                    &request.url,
                ))
            {
                return Err(error("TypeError"));
            }
            let headers = platform_object(&state, request.headers_id)?;
            let PlatformObjectRecord::Headers(headers) = &*headers else {
                return Err(error("TypeError"));
            };
            (
                PlatformHttpRequest {
                    upgrade_mixed_image: false,
                    fetch: Some(crate::platform::FetchOptions {
                        keepalive: true,
                        privileged_range: request.privileged_range,
                        cache: request.cache,
                        mode: request.mode.clone(),
                        redirect: request.redirect.clone(),
                        integrity: request.integrity.clone(),
                    }),
                    use_cors_preflight: false,
                    method: request.method.clone(),
                    url: request.url.clone(),
                    headers: headers.entries.clone(),
                    body: request.body.clone(),
                    credentials: request.credentials.clone(),
                    initiator_url: if request.referrer.is_empty() {
                        Some(String::new())
                    } else if request.referrer == "client" {
                        request_initiator_url(scope)
                    } else {
                        Some(resolve_platform_url(scope, &request.referrer)?)
                    },
                    referrer_policy: Some(if request.referrer.is_empty() {
                        "no-referrer".into()
                    } else if request.referrer_policy.is_empty() {
                        state.document_referrer_policy()
                    } else {
                        request.referrer_policy.clone()
                    }),
                },
                request.signal_id,
            )
        };
        if matches!(&*platform_object(&state,signal)?,PlatformObjectRecord::AbortSignal(signal)if signal.aborted)
        {
            return Err(error("AbortError"));
        }
        if crate::fetch_later::request_bytes(&request).is_none_or(|bytes| bytes > 65536) {
            return Err(error("QuotaExceededError"));
        }
        if state.execution_retired()
            || state.execution_suspended.get()
            || generation != state.document_generation()
            || authority != state.document_authority_epoch()
        {
            return Err(error("InvalidStateError"));
        }
        let request_root = new_platform_object(scope, &state, "Request", request_id)?;
        let signal_root = new_platform_object(scope, &state, "AbortSignal", signal)?;
        platform_listener_graph::refresh(scope, &state, request_id)?;
        let backend = if let Some(provider) = state.deferred_fetch_backend_provider.get() {
            provider(scope, &state)?
        } else {
            state
                .deferred_fetch_backend
                .borrow()
                .clone()
                .ok_or_else(|| error("InvalidStateError"))?
        };
        let sequence = {
            let mut registry = state.deferred_fetch.borrow_mut();
            let next = registry
                .token_sequence
                .checked_add(1)
                .ok_or_else(|| error("QuotaExceededError"))?;
            registry.token_sequence = next;
            next
        };
        let mut bytes = vec![0; 32];
        bytes[..8].copy_from_slice(&sequence.to_be_bytes());
        getrandom::fill(&mut bytes[8..]).map_err(|_| error("InvalidStateError"))?;
        let token = DeferredFetchToken(bytes);
        // Allocate the observable result before native admission: allocation failure
        // cannot leave an unobservable admitted request behind.
        let object = v8::Object::new(scope);
        let mut registry = state.deferred_fetch.borrow_mut();
        if registry.records.len() >= MAX_RECORDS {
            return Err(error("QuotaExceededError"));
        }
        let id = registry
            .next
            .checked_add(1)
            .ok_or_else(|| error("QuotaExceededError"))?;
        let prototype = registry
            .prototype
            .as_ref()
            .ok_or_else(|| error("InvalidStateError"))?;
        let prototype = prototype
            .to_local(scope)
            .ok_or_else(|| error("InvalidStateError"))?;
        if object.set_prototype(scope, prototype.into()) != Some(true) {
            return Err(V8BindingError::JavaScriptException);
        }
        let brand = key(scope);
        let value = v8::BigInt::new_from_u64(scope, id);
        if object.set_private(scope, brand, value.into()) != Some(true) {
            return Err(V8BindingError::JavaScriptException);
        }
        if let Err(err) = backend.prepare(
            &token,
            request,
            deadline.map(|deadline| deadline.saturating_duration_since(Instant::now())),
        ) {
            // Admission can succeed remotely even if its reply is lost. Keep cleanup
            // custody independently of an author-visible result until cancellation ACK.
            let cancellation = backend.cancel(&token);
            let pending = matches!(
                &cancellation,
                Err(DeferredFetchError::Transport(_)) | Ok(DeferredFetchState::Pending)
            );
            let ack_pending = !pending
                && matches!(
                    backend.acknowledge(&token),
                    Err(DeferredFetchError::Transport(_))
                );
            if pending || ack_pending {
                registry.next = id;
                registry.records.insert(
                    id,
                    Record {
                        request_root: pending.then(|| v8::Global::new(scope, request_root)),
                        signal_root: pending.then(|| v8::Global::new(scope, signal_root)),
                        object: v8::Weak::new(scope, object),
                        token,
                        backend,
                        signal,
                        deadline: Instant::now().checked_add(Duration::from_millis(25)),
                        action: if pending {
                            Action::Cancel
                        } else {
                            Action::Done
                        },
                        activated: matches!(cancellation, Ok(DeferredFetchState::Activated)),
                    },
                );
            }
            return Err(backend_error(err));
        }
        registry.next = id;
        registry.records.insert(
            id,
            Record {
                request_root: Some(v8::Global::new(scope, request_root)),
                signal_root: Some(v8::Global::new(scope, signal_root)),
                object: v8::Weak::new(scope, object),
                token,
                backend,
                signal,
                deadline,
                action: Action::Waiting,
                activated: false,
            },
        );
        Ok(object)
    })();
    if scope.has_caught() || scope.is_execution_terminating() {
        scope.rethrow();
        return;
    }
    match result {
        Ok(object) => rv.set(object.into()),
        Err(err) => {
            throw_binding_error(scope, &err);
            scope.rethrow();
        }
    }
}
pub(super) fn install(
    scope: &mut v8::PinScope,
    context: v8::Local<v8::Context>,
    state: &V8BindingState,
) -> Result<(), V8BindingError> {
    let constructor = v8::Function::new(scope, illegal).ok_or_else(|| error("TypeError"))?;
    let name = v8::String::new(scope, "prototype").unwrap();
    let prototype = constructor
        .get(scope, name.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(|| error("TypeError"))?;
    let getter = v8::Function::new(scope, activated).ok_or_else(|| error("TypeError"))?;
    let undefined = v8::undefined(scope);
    let mut descriptor = v8::PropertyDescriptor::new_from_get_set(getter.into(), undefined.into());
    descriptor.set_enumerable(true);
    descriptor.set_configurable(true);
    let name = v8::String::new(scope, "activated").unwrap();
    prototype.define_property(scope, name.into(), &descriptor);
    let tag = v8::Symbol::get_to_string_tag(scope);
    let name = v8::String::new(scope, "FetchLaterResult").unwrap();
    constructor.set_name(name);
    prototype.define_own_property(
        scope,
        tag.into(),
        name.into(),
        v8::PropertyAttribute::READ_ONLY | v8::PropertyAttribute::DONT_ENUM,
    );
    context
        .global(scope)
        .create_data_property(scope, name.into(), constructor.into());
    let prototype = context_intrinsics::retain_object_for(
        scope,
        state,
        "fetch-later-result-prototype",
        prototype,
    )?;
    state.deferred_fetch.borrow_mut().prototype = Some(prototype);
    if state.secure_context.get() {
        let function = v8::Function::builder(schedule)
            .length(1)
            .constructor_behavior(v8::ConstructorBehavior::Throw)
            .build(scope)
            .ok_or_else(|| error("TypeError"))?;
        let name = v8::String::new(scope, "fetchLater").unwrap();
        function.set_name(name);
        context
            .global(scope)
            .create_data_property(scope, name.into(), function.into());
    }
    Ok(())
}
fn process(record: &mut Record, now: Instant) {
    if record.action == Action::Waiting && record.deadline.is_some_and(|deadline| deadline <= now) {
        record.action = Action::Activate;
    }
    let result = match record.action {
        Action::Activate => record.backend.activate(&record.token),
        Action::Cancel => record.backend.cancel(&record.token),
        _ => return,
    };
    match result {
        Ok(DeferredFetchState::Pending) => {
            record.deadline = now.checked_add(Duration::from_millis(25));
        }
        Ok(status) => {
            record.activated = status == DeferredFetchState::Activated;
            record.action = Action::Done;
            record.deadline = None;
            record.request_root = None;
            record.signal_root = None;
        }
        Err(DeferredFetchError::Transport(_)) => {
            record.deadline = now.checked_add(Duration::from_millis(25));
        }
        // Expired or invalid capabilities cannot acquire custody on retry. Release
        // local roots without inventing a successful activation acknowledgement.
        Err(DeferredFetchError::Type(_) | DeferredFetchError::Quota(_)) => {
            record.action = Action::Done;
            record.deadline = None;
            record.request_root = None;
            record.signal_root = None;
        }
    }
}
// A missing deadline is insufficient: retained records still refresh/finalize.
pub(super) fn records_empty(state: &V8BindingState) -> bool {
    state.deferred_fetch.borrow().records.is_empty()
}
pub(super) fn poll(scope: &mut v8::PinScope, state: &V8BindingState) -> Result<(), V8BindingError> {
    let now = Instant::now();
    for record in state.deferred_fetch.borrow_mut().records.values_mut() {
        if record.deadline.is_some_and(|deadline| deadline <= now) {
            process(record, now)
        }
    }
    refresh_all(state);
    finalize(scope, state)
}
pub(super) fn deactivate(state: &V8BindingState) {
    for record in state.deferred_fetch.borrow_mut().records.values_mut() {
        if record.action == Action::Waiting {
            record.action = Action::Activate;
        }
        process(record, Instant::now());
    }
}
pub(super) fn abort(state: &V8BindingState, signal: u64) {
    for record in state.deferred_fetch.borrow_mut().records.values_mut() {
        if record.signal == signal && record.action == Action::Waiting {
            record.action = Action::Cancel;
            process(record, Instant::now());
        }
    }
}
pub(super) fn delay(state: &V8BindingState) -> Option<Duration> {
    let now = Instant::now();
    state
        .deferred_fetch
        .borrow()
        .records
        .values()
        .filter_map(|record| {
            record
                .deadline
                .map(|deadline| deadline.saturating_duration_since(now))
        })
        .min()
}
pub(super) fn ready(state: &V8BindingState) -> bool {
    delay(state).is_some_and(|delay| delay.is_zero())
}

#[cfg(test)]
mod tests {
    use super::super::tests::initialize_v8;
    use super::*;
    use oryn_dom::html;
    #[derive(Default)]
    struct Backend {
        prepared: RefCell<Vec<PlatformHttpRequest>>,
        activated: RefCell<BTreeSet<DeferredFetchToken>>,
        cancelled: RefCell<BTreeSet<DeferredFetchToken>>,
        fail: Cell<bool>,
        ack_fail: Cell<bool>,
        state_fail: Cell<bool>,
        expired: Cell<bool>,
        deadlines: RefCell<Vec<Option<Duration>>>,
    }
    impl DeferredFetchBackend for Backend {
        fn prepare(
            &self,
            _token: &DeferredFetchToken,
            request: PlatformHttpRequest,
            after: Option<Duration>,
        ) -> Result<(), DeferredFetchError> {
            self.deadlines.borrow_mut().push(after);
            self.prepared.borrow_mut().push(request);
            Ok(())
        }
        fn activate(
            &self,
            token: &DeferredFetchToken,
        ) -> Result<DeferredFetchState, DeferredFetchError> {
            if self.expired.get() {
                return Err(DeferredFetchError::Type("capability expired".into()));
            }
            self.activated.borrow_mut().insert(token.clone());
            if self.fail.replace(false) {
                self.state_fail.set(true);
                Err(DeferredFetchError::Transport("lost ACK".into()))
            } else {
                Ok(DeferredFetchState::Activated)
            }
        }
        fn cancel(
            &self,
            token: &DeferredFetchToken,
        ) -> Result<DeferredFetchState, DeferredFetchError> {
            if self.activated.borrow().contains(token) {
                return Ok(DeferredFetchState::Activated);
            }
            self.cancelled.borrow_mut().insert(token.clone());
            Ok(DeferredFetchState::Canceled)
        }
        fn acknowledge(&self, _token: &DeferredFetchToken) -> Result<(), DeferredFetchError> {
            if self.ack_fail.get() {
                Err(DeferredFetchError::Transport(
                    "lost terminal observation ACK".into(),
                ))
            } else {
                Ok(())
            }
        }
        fn state(
            &self,
            token: &DeferredFetchToken,
        ) -> Result<DeferredFetchState, DeferredFetchError> {
            if self.state_fail.replace(false) {
                return Err(DeferredFetchError::Transport("lost status ACK".into()));
            }
            if self.expired.get() {
                return Err(DeferredFetchError::Type("capability expired".into()));
            }
            Ok(if self.activated.borrow().contains(token) {
                DeferredFetchState::Activated
            } else if self.cancelled.borrow().contains(token) {
                DeferredFetchState::Canceled
            } else {
                DeferredFetchState::Pending
            })
        }
    }
    fn eval(scope: &mut v8::PinScope, source: &str) {
        let source = v8::String::new(scope, source).unwrap();
        let script = v8::Script::compile(scope, source, None).unwrap();
        assert!(script.run(scope).is_some(), "{source:?}");
    }
    #[test]
    fn result_timer_abort_and_deactivation_share_one_native_capability() {
        initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        context.set_slot(Rc::new(crate::realm::SecureContextState(true)));
        let dom = Rc::new(RefCell::new(
            html::parse("<body></body>").into_runtime().unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(880), 1).unwrap();
        let backend = Rc::new(Backend::default());
        state.set_deferred_fetch_backend(backend.clone());
        eval(
            scope,
            r#"globalThis.assert=v=>{if(!v)throw Error('assert')};globalThis.c=new AbortController();globalThis.a=fetchLater('https://example.test/a',{method:'POST',body:'payload',activateAfter:0});globalThis.b=fetchLater('https://example.test/b',{activateAfter:60000});globalThis.d=fetchLater('https://example.test/d',{signal:c.signal});assert(!a.activated&&!b.activated&&!d.activated);c.abort();"#,
        );
        assert_eq!(backend.cancelled.borrow().len(), 1);
        assert!(backend.activated.borrow().is_empty());
        poll(scope, &state).unwrap();
        assert_eq!(backend.activated.borrow().len(), 1);
        eval(scope, "assert(a.activated&&!b.activated&&!d.activated)");
        state.set_execution_suspended(true);
        assert_eq!(backend.activated.borrow().len(), 2);
        state.set_execution_suspended(false);
        state.retire_execution();
        assert_eq!(backend.activated.borrow().len(), 2);
        eval(scope, "assert(a.activated&&b.activated&&!d.activated)");
        assert_eq!(backend.prepared.borrow()[0].body, b"payload");
    }
    #[test]
    fn conversion_errors_do_not_admit_and_result_brand_is_native() {
        initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        context.set_slot(Rc::new(crate::realm::SecureContextState(true)));
        let dom = Rc::new(RefCell::new(
            html::parse("<body></body>").into_runtime().unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(881), 1).unwrap();
        let backend = Rc::new(Backend::default());
        state.set_deferred_fetch_backend(backend.clone());
        eval(
            scope,
            r#"const assert=v=>{if(!v)throw Error('assert')};function fails(name,fn){try{fn()}catch(e){assert(e.name===name);return}throw Error('missing exception')};fails('TypeError',()=>fetchLater());let marker={};try{fetchLater('https://example.test',{get method(){throw marker}})}catch(e){assert(e===marker)};fails('TypeError',()=>fetchLater('http://example.test'));fails('TypeError',()=>fetchLater('data:text/plain,x'));fails('RangeError',()=>fetchLater('https://example.test',{activateAfter:-1}));fails('TypeError',()=>fetchLater('https://example.test',{activateAfter:NaN}));fails('TypeError',()=>fetchLater('https://example.test',{method:'POST',body:new ReadableStream(),duplex:'half'}));fails('QuotaExceededError',()=>fetchLater('https://example.test',{method:'POST',body:'x'.repeat(65536)}));let c=new AbortController();c.abort();fails('AbortError',()=>fetchLater('https://example.test',{signal:c.signal}));fails('TypeError',()=>new FetchLaterResult());fails('TypeError',()=>Object.getOwnPropertyDescriptor(FetchLaterResult.prototype,'activated').get.call({}));globalThis.result=fetchLater('https://example.test');fails('TypeError',()=>{ 'use strict';result.activated=true });assert(result instanceof FetchLaterResult&&!result.activated);"#,
        );
        assert_eq!(backend.prepared.borrow().len(), 1);
    }
    #[test]
    fn lost_activation_ack_retries_same_token_without_regressing_result() {
        initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        context.set_slot(Rc::new(crate::realm::SecureContextState(true)));
        let dom = Rc::new(RefCell::new(
            html::parse("<body></body>").into_runtime().unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(882), 1).unwrap();
        let backend = Rc::new(Backend::default());
        backend.fail.set(true);
        state.set_deferred_fetch_backend(backend.clone());
        eval(
            scope,
            "globalThis.result=fetchLater('https://example.test',{activateAfter:0})",
        );
        poll(scope, &state).unwrap();
        assert_eq!(backend.activated.borrow().len(), 1);
        assert!(delay(&state).is_some());
        for record in state.deferred_fetch.borrow_mut().records.values_mut() {
            record.deadline = Some(Instant::now());
        }
        poll(scope, &state).unwrap();
        assert!(delay(&state).is_none());
        assert_eq!(backend.activated.borrow().len(), 1);
        eval(
            scope,
            "if(!result.activated)throw Error('activation not published')",
        );
    }
    #[test]
    fn dropped_result_keeps_abort_custody_and_conversion_cannot_revive_retired_document() {
        initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        context.set_slot(Rc::new(crate::realm::SecureContextState(true)));
        let dom = Rc::new(RefCell::new(
            html::parse("<body></body>").into_runtime().unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(883), 1).unwrap();
        let backend = Rc::new(Backend::default());
        state.set_deferred_fetch_backend(backend.clone());
        eval(
            scope,
            "globalThis.controller=new AbortController();fetchLater('https://example.test',{signal:controller.signal})",
        );
        scope.low_memory_notification();
        state.process_weak_releases().unwrap();
        eval(scope, "controller.abort()");
        assert_eq!(backend.cancelled.borrow().len(), 1);
        assert!(
            state
                .deferred_fetch
                .borrow()
                .records
                .values()
                .all(|record| record.signal_root.is_none() && record.request_root.is_none())
        );
        fn retire(
            scope: &mut v8::PinScope,
            _args: v8::FunctionCallbackArguments,
            _rv: v8::ReturnValue,
        ) {
            scope
                .get_current_context()
                .get_slot::<V8BindingState>()
                .unwrap()
                .retire_execution();
        }
        let function = v8::Function::new(scope, retire).unwrap();
        let name = v8::String::new(scope, "retireNative").unwrap();
        context
            .global(scope)
            .set(scope, name.into(), function.into());
        eval(
            scope,
            "let rejected=false;try{fetchLater('https://example.test',{get activateAfter(){retireNative();return 0}})}catch(e){rejected=e.name==='InvalidStateError'}if(!rejected)throw Error('retirement escaped')",
        );
        assert_eq!(backend.prepared.borrow().len(), 1);
    }
    #[test]
    fn repeated_lost_admission_and_cancel_ack_is_bounded_and_retirement_preserves_cleanup() {
        struct Failed {
            prepares: Cell<usize>,
            recover: Cell<bool>,
            sequences: RefCell<Vec<u64>>,
        }
        impl DeferredFetchBackend for Failed {
            fn prepare(
                &self,
                token: &DeferredFetchToken,
                _request: PlatformHttpRequest,
                _after: Option<Duration>,
            ) -> Result<(), DeferredFetchError> {
                self.sequences
                    .borrow_mut()
                    .push(u64::from_be_bytes(token.0[..8].try_into().unwrap()));
                self.prepares.set(self.prepares.get() + 1);
                Err(DeferredFetchError::Transport("lost admission ACK".into()))
            }
            fn activate(
                &self,
                _token: &DeferredFetchToken,
            ) -> Result<DeferredFetchState, DeferredFetchError> {
                panic!("unreturned admission must only cancel")
            }
            fn cancel(
                &self,
                _token: &DeferredFetchToken,
            ) -> Result<DeferredFetchState, DeferredFetchError> {
                if self.recover.get() {
                    Ok(DeferredFetchState::Canceled)
                } else {
                    Err(DeferredFetchError::Transport("lost cancel ACK".into()))
                }
            }
            fn acknowledge(&self, _token: &DeferredFetchToken) -> Result<(), DeferredFetchError> {
                Ok(())
            }
            fn state(
                &self,
                _token: &DeferredFetchToken,
            ) -> Result<DeferredFetchState, DeferredFetchError> {
                Err(DeferredFetchError::Transport("lost state reply".into()))
            }
        }
        initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        context.set_slot(Rc::new(crate::realm::SecureContextState(true)));
        let dom = Rc::new(RefCell::new(
            html::parse("<body></body>").into_runtime().unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(884), 1).unwrap();
        let backend = Rc::new(Failed {
            prepares: Cell::new(0),
            recover: Cell::new(false),
            sequences: RefCell::new(Vec::new()),
        });
        state.set_deferred_fetch_backend(backend.clone());
        eval(
            scope,
            &format!(
                "for(let i=0;i<{};i++){{try{{fetchLater('https://example.test')}}catch(e){{if(e.name!==(i<{}?'InvalidStateError':'QuotaExceededError'))throw e}}}}",
                MAX_RECORDS + 3,
                MAX_RECORDS
            ),
        );
        assert_eq!(backend.prepares.get(), MAX_RECORDS);
        assert_eq!(
            *backend.sequences.borrow(),
            (1..=MAX_RECORDS as u64).collect::<Vec<_>>()
        );
        assert_eq!(state.deferred_fetch.borrow().records.len(), MAX_RECORDS);
        state.retire_execution();
        assert!(
            state
                .deferred_fetch
                .borrow()
                .records
                .values()
                .all(|record| record.action == Action::Cancel)
        );
        backend.recover.set(true);
        deactivate(&state);
        assert!(
            state
                .deferred_fetch
                .borrow()
                .records
                .values()
                .all(|record| record.action == Action::Done
                    && record.signal_root.is_none()
                    && record.request_root.is_none())
        );
    }

    #[test]
    fn terminal_native_expiry_releases_roots_without_success_or_retry() {
        initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        context.set_slot(Rc::new(crate::realm::SecureContextState(true)));
        let dom = Rc::new(RefCell::new(
            html::parse("<body></body>").into_runtime().unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(885), 1).unwrap();
        let backend = Rc::new(Backend::default());
        state.set_deferred_fetch_backend(backend.clone());
        eval(
            scope,
            "globalThis.result=fetchLater('https://example.test',{activateAfter:0})",
        );
        backend.expired.set(true);
        poll(scope, &state).unwrap();
        assert!(delay(&state).is_none());
        assert!(
            state
                .deferred_fetch
                .borrow()
                .records
                .values()
                .all(|record| record.action == Action::Done
                    && record.request_root.is_none()
                    && record.signal_root.is_none())
        );
        assert!(backend.activated.borrow().is_empty());
        eval(
            scope,
            "if(result.activated)throw Error('fabricated native ACK')",
        );
        deactivate(&state);
        assert!(backend.activated.borrow().is_empty());
    }

    #[test]
    fn retained_completed_results_do_not_consume_pending_capacity() {
        initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        context.set_slot(Rc::new(crate::realm::SecureContextState(true)));
        let dom = Rc::new(RefCell::new(
            html::parse("<body></body>").into_runtime().unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(886), 1).unwrap();
        let backend = Rc::new(Backend::default());
        state.set_deferred_fetch_backend(backend.clone());
        eval(scope, "globalThis.results=[]");
        for _ in 0..MAX_RECORDS + 16 {
            eval(
                scope,
                "results.push(fetchLater('https://example.test',{activateAfter:0}))",
            );
            poll(scope, &state).unwrap();
            assert!(state.deferred_fetch.borrow().records.is_empty());
        }
        assert_eq!(backend.activated.borrow().len(), MAX_RECORDS + 16);
        eval(
            scope,
            &format!(
                "if(results.length!=={}||results.some(result=>!result.activated))throw Error('terminal metadata lost')",
                MAX_RECORDS + 16
            ),
        );
        eval(
            scope,
            "globalThis.cancel=new AbortController();globalThis.cancelled=fetchLater('https://example.test',{signal:cancel.signal});cancel.abort();if(cancelled.activated)throw Error('cancelled activated')",
        );
        assert!(state.deferred_fetch.borrow().records.is_empty());
    }

    #[test]
    fn request_keepalive_preserves_conversion_clone_and_rejects_inherited_stream_source() {
        initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        context.set_slot(Rc::new(crate::realm::SecureContextState(true)));
        let dom = Rc::new(RefCell::new(
            html::parse("<body></body>").into_runtime().unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(889), 1).unwrap();
        let backend = Rc::new(Backend::default());
        state.set_deferred_fetch_backend(backend.clone());
        eval(
            scope,
            r#"const request=new Request('https://example.test',{method:'POST',body:'bytes',keepalive:true});if(!request.keepalive||!request.clone().keepalive||!new Request(request.clone(),{}).keepalive||new Request(request.clone(),{keepalive:false}).keepalive)throw Error('keepalive metadata');const streamRequest=new Request('https://example.test',{method:'POST',body:new ReadableStream(),duplex:'half'});let caught=false;try{new Request(streamRequest,{keepalive:true})}catch(e){caught=e instanceof TypeError}if(!caught||streamRequest.bodyUsed)throw Error('inherited stream accepted or consumed');fetchLater('https://example.test',{activateAfter:0})"#,
        );
        assert!(
            backend.prepared.borrow()[0]
                .fetch
                .as_ref()
                .unwrap()
                .keepalive
        );
    }

    #[test]
    fn terminal_observation_precedes_ack_and_lost_ack_retains_only_cleanup() {
        initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        context.set_slot(Rc::new(crate::realm::SecureContextState(true)));
        let dom = Rc::new(RefCell::new(
            html::parse("<body></body>").into_runtime().unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(888), 1).unwrap();
        let backend = Rc::new(Backend::default());
        backend.ack_fail.set(true);
        state.set_deferred_fetch_backend(backend.clone());
        eval(
            scope,
            "globalThis.result=fetchLater('https://example.test',{activateAfter:0})",
        );
        poll(scope, &state).unwrap();
        assert_eq!(state.deferred_fetch.borrow().records.len(), 1);
        assert!(
            state
                .deferred_fetch
                .borrow()
                .records
                .values()
                .all(|record| record.action == Action::Done
                    && record.request_root.is_none()
                    && record.signal_root.is_none())
        );
        eval(
            scope,
            "if(!result.activated)throw Error('result not durable before ACK')",
        );
        assert!(delay(&state).is_some());
        backend.ack_fail.set(false);
        poll(scope, &state).unwrap();
        assert!(state.deferred_fetch.borrow().records.is_empty());
        eval(
            scope,
            "if(!result.activated)throw Error('result lost after ACK')",
        );
        assert_eq!(backend.activated.borrow().len(), 1);
    }

    #[test]
    fn parent_timer_is_observable_without_js_poll_and_wins_late_abort() {
        initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        v8::scope!(let scope,&mut isolate);
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        context.set_slot(Rc::new(crate::realm::SecureContextState(true)));
        let dom = Rc::new(RefCell::new(
            html::parse("<body></body>").into_runtime().unwrap(),
        ));
        let state = install_generated_bindings(scope, context, dom, RealmId(887), 1).unwrap();
        let backend = Rc::new(Backend::default());
        state.set_deferred_fetch_backend(backend.clone());
        eval(
            scope,
            "globalThis.controller=new AbortController();globalThis.result=fetchLater('https://example.test',{signal:controller.signal,activateAfter:60000})",
        );
        assert!(backend.deadlines.borrow()[0].is_some_and(|delay| delay
            <= Duration::from_secs(60)
            && delay > Duration::from_secs(59)));
        let token = state
            .deferred_fetch
            .borrow()
            .records
            .values()
            .next()
            .unwrap()
            .token
            .clone();
        backend.activated.borrow_mut().insert(token);
        eval(
            scope,
            "controller.abort();if(!result.activated)throw Error('parent activation lost to JS abort')",
        );
        assert!(backend.cancelled.borrow().is_empty());
        assert!(state.deferred_fetch.borrow().records.is_empty());
        eval(
            scope,
            "globalThis.other=fetchLater('https://example.test',{activateAfter:60000})",
        );
        let token = state
            .deferred_fetch
            .borrow()
            .records
            .values()
            .next()
            .unwrap()
            .token
            .clone();
        backend.activated.borrow_mut().insert(token);
        eval(
            scope,
            "if(!other.activated)throw Error('getter did not observe parent timer')",
        );
        assert!(state.deferred_fetch.borrow().records.is_empty());
    }
}

#[cfg(test)]
mod prototype_retention_tests {
    use super::*;
    #[test]
    fn original_result_prototype_survives_replacement_then_releases_retired_context() {
        super::super::tests::initialize_v8();
        let mut isolate = v8::Isolate::new(Default::default());
        let (context, prototype, anchor, state) = {
            v8::scope!(let scope, &mut isolate);
            let context = v8::Context::new(scope, Default::default());
            let scope = &mut v8::ContextScope::new(scope, context);
            let dom = Rc::new(RefCell::new(DomRuntime::new()));
            let state = install_generated_bindings(scope, context, dom, RealmId(948), 1).unwrap();
            let prototype = state
                .deferred_fetch
                .borrow()
                .prototype
                .as_ref()
                .unwrap()
                .to_local(scope)
                .unwrap();
            let key = v8::String::new(scope, "FetchLaterResult").unwrap();
            let replacement = v8::Object::new(scope);
            context
                .global(scope)
                .set(scope, key.into(), replacement.into());
            let anchor = v8::Object::new(scope);
            assert_eq!(anchor.set_prototype(scope, prototype.into()), Some(true));
            state.retire_execution();
            state.refresh_connected_wrapper_roots(scope);
            (
                v8::Weak::new(scope, context),
                v8::Weak::new(scope, prototype),
                v8::Global::new(scope, anchor),
                state,
            )
        };
        isolate.low_memory_notification();
        isolate.low_memory_notification();
        {
            v8::scope!(let scope, &mut isolate);
            assert!(context.to_local(scope).is_some());
            let retained_context = context.to_local(scope).unwrap();
            let scope = &mut v8::ContextScope::new(scope, retained_context);
            let original = prototype.to_local(scope).unwrap();
            let anchor = v8::Local::new(scope, &anchor);
            assert!(
                anchor
                    .get_prototype(scope)
                    .unwrap()
                    .strict_equals(original.into())
            );
            assert!(
                state
                    .deferred_fetch
                    .borrow()
                    .prototype
                    .as_ref()
                    .unwrap()
                    .to_local(scope)
                    .unwrap()
                    .strict_equals(original.into())
            );
        }
        drop(anchor);
        isolate.low_memory_notification();
        isolate.low_memory_notification();
        v8::scope!(let scope, &mut isolate);
        assert!(context.to_local(scope).is_none());
        assert!(prototype.to_local(scope).is_none());
        assert!(state.deferred_fetch.borrow().records.is_empty());
    }
}
