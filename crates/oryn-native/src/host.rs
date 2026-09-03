#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKind {
    RawV8,
    DenoCore,
}

/// Thread-affine JavaScript host owned by one `PageRuntime`.
///
/// This trait intentionally has no `Send` bound. Typed page handles cross
/// threads; V8 isolates and persistent handles never do.
pub trait JavaScriptHost {
    fn kind(&self) -> HostKind;
    fn execute(&mut self, source: &str) -> Result<(), HostError>;
    fn evaluate_string(&mut self, source: &str) -> Result<String, HostError>;
    fn terminate(&mut self);
}

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("JavaScript host is not enabled in this build")]
    Disabled,
    #[error("JavaScript execution failed: {0}")]
    Execution(String),
    #[error("JavaScript exceeded the configured {resource} limit")]
    LimitExceeded { resource: String },
}

pub struct DisabledHost;

impl JavaScriptHost for DisabledHost {
    fn kind(&self) -> HostKind {
        HostKind::RawV8
    }

    fn execute(&mut self, _source: &str) -> Result<(), HostError> {
        Err(HostError::Disabled)
    }

    fn evaluate_string(&mut self, _source: &str) -> Result<String, HostError> {
        Err(HostError::Disabled)
    }

    fn terminate(&mut self) {}
}

#[cfg(feature = "v8-host")]
mod raw_v8 {
    use std::{
        borrow::Cow,
        cell::RefCell,
        collections::{BTreeMap, BTreeSet},
        rc::Rc,
        sync::{
            Arc, Mutex, Once,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        time::{Duration, Instant},
    };

    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
    use serde::Serialize;
    use url::Url;

    use super::{HostError, HostKind, JavaScriptHost};
    #[cfg(unix)]
    use crate::context_service::{
        ContextServiceRequest, ContextServiceResponse, HttpStreamRequest, SharedContextClient,
    };
    use crate::network::{NetworkRequest, SharedNetworkBroker, collect_operation_bounded};
    #[cfg(unix)]
    use oryn_common::v3::PageId;
    use oryn_dom::NodeId;
    #[cfg(unix)]
    use oryn_js::platform::{
        HostOperationId, IdbHostResponse, PlatformHost, PlatformHostCompletion,
        PlatformHostRequest, PlatformHostResult, PlatformWebSocketFrame,
    };
    use oryn_js::realm::{RealmDescriptor, RealmId, RealmRegistry};
    use oryn_js::scheduler::{PageScheduler, Task, TaskSource};
    use oryn_js::v8_bindings::{
        BindingLayout, BindingLayoutProvider, SharedDom, V8BindingState,
        deliver_mutation_observers, deliver_selection_change, install_generated_bindings,
        install_generated_bindings_with_root, publish_named_window_properties, wrap_node,
    };

    static INITIALIZE_V8: Once = Once::new();

    pub const STARTUP_EXTERNAL_REFERENCE_NAMES: &[&str] = &[
        "fetch_async_callback",
        "context_callback",
        "stream_read_async_callback",
        "crypto_callback",
        "timer_schedule_callback",
        "timer_cancel_callback",
        "dedicated_worker_create_callback",
        "dedicated_worker_post_callback",
        "dedicated_worker_take_callback",
        "dedicated_worker_terminate_callback",
        "dedicated_worker_emit_callback",
        "dedicated_worker_close_callback",
    ];

    pub fn startup_external_references() -> Cow<'static, [v8::ExternalReference]> {
        use v8::MapFnTo;
        vec![
            v8::ExternalReference {
                function: fetch_async_callback.map_fn_to(),
            },
            v8::ExternalReference {
                function: context_callback.map_fn_to(),
            },
            v8::ExternalReference {
                function: stream_read_async_callback.map_fn_to(),
            },
            v8::ExternalReference {
                function: crypto_callback.map_fn_to(),
            },
            v8::ExternalReference {
                function: timer_schedule_callback.map_fn_to(),
            },
            v8::ExternalReference {
                function: timer_cancel_callback.map_fn_to(),
            },
            v8::ExternalReference {
                function: dedicated_worker_create_callback.map_fn_to(),
            },
            v8::ExternalReference {
                function: dedicated_worker_post_callback.map_fn_to(),
            },
            v8::ExternalReference {
                function: dedicated_worker_take_callback.map_fn_to(),
            },
            v8::ExternalReference {
                function: dedicated_worker_terminate_callback.map_fn_to(),
            },
            v8::ExternalReference {
                function: dedicated_worker_emit_callback.map_fn_to(),
            },
            v8::ExternalReference {
                function: dedicated_worker_close_callback.map_fn_to(),
            },
        ]
        .into()
    }

    #[cfg(feature = "v8-sandbox")]
    unsafe extern "C" {
        fn v8__V8__IsSandboxEnabled() -> bool;
    }

    #[cfg(feature = "v8-sandbox")]
    fn assert_v8_heap_sandbox() {
        assert!(
            unsafe { v8__V8__IsSandboxEnabled() },
            "linked V8 archive has the heap sandbox disabled"
        );
    }

    pub struct RawV8Host {
        isolate: v8::OwnedIsolate,
        contexts: BTreeMap<RealmId, v8::Global<v8::Context>>,
        security_tokens: BTreeMap<String, v8::Global<v8::String>>,
        realms: RealmRegistry,
        active_realm: RealmId,
        binding_states: BTreeMap<RealmId, Rc<V8BindingState>>,
        trusted_types_config: Option<(oryn_js::trusted_types::TrustedTypesConfig, bool)>,
        execution_wall_limit: Duration,
        isolate_entered: bool,
    }

    struct ExecutionDeadline {
        cancel: Option<mpsc::Sender<()>>,
        expired: Arc<AtomicBool>,
    }

    impl ExecutionDeadline {
        fn arm(isolate: &v8::OwnedIsolate, limit: Duration) -> Self {
            let handle = isolate.thread_safe_handle();
            let (sender, receiver) = mpsc::channel();
            let expired = Arc::new(AtomicBool::new(false));
            let thread_expired = expired.clone();
            std::thread::spawn(move || {
                if receiver.recv_timeout(limit).is_err() {
                    thread_expired.store(true, Ordering::Release);
                    handle.terminate_execution();
                }
            });
            Self {
                cancel: Some(sender),
                expired,
            }
        }

        fn expired(&self) -> bool {
            self.expired.load(Ordering::Acquire)
        }
    }

    impl Drop for ExecutionDeadline {
        fn drop(&mut self) {
            if let Some(sender) = self.cancel.take() {
                let _ = sender.send(());
            }
        }
    }

    #[derive(Clone)]
    struct ModuleSources(BTreeMap<String, String>);

    #[derive(Default)]
    struct ModuleNames(BTreeMap<i32, String>);

    #[derive(Default)]
    struct ModuleCache(BTreeMap<String, v8::Global<v8::Module>>);

    #[derive(Default)]
    struct ModuleResolutionFailures(Vec<String>);

    #[derive(Default)]
    struct PromiseRejections(BTreeMap<i32, String>);

    #[derive(Clone)]
    struct FetchState {
        broker: SharedNetworkBroker,
        base_url: Url,
    }

    #[cfg(unix)]
    #[derive(Clone)]
    struct ContextState {
        client: SharedContextClient,
        page_id: PageId,
    }

    #[cfg(unix)]
    struct ContextPlatformHost {
        context: ContextState,
        sender: mpsc::Sender<PlatformHostCompletion>,
        receiver: Mutex<mpsc::Receiver<PlatformHostCompletion>>,
        idb_sender: mpsc::Sender<(HostOperationId, PlatformHostRequest)>,
        cancellable_http_starts: Arc<Mutex<BTreeSet<HostOperationId>>>,
    }

    #[cfg(unix)]
    impl ContextPlatformHost {
        fn new(context: ContextState) -> Self {
            let (sender, receiver) = mpsc::channel();
            let (idb_sender, idb_receiver) = mpsc::channel();
            let idb_context = context.clone();
            let idb_completions = sender.clone();
            std::thread::Builder::new()
                .name("oryn-platform-indexeddb".into())
                .spawn(move || {
                    while let Ok((operation_id, request)) = idb_receiver.recv() {
                        let result = execute_platform_request(&idb_context, operation_id, request);
                        if idb_completions
                            .send(PlatformHostCompletion {
                                operation_id,
                                result,
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                })
                .expect("spawn ordered IndexedDB platform service");
            Self {
                context,
                sender,
                receiver: Mutex::new(receiver),
                idb_sender,
                cancellable_http_starts: Arc::new(Mutex::new(BTreeSet::new())),
            }
        }
    }

    #[cfg(unix)]
    impl PlatformHost for ContextPlatformHost {
        fn submit(
            &self,
            operation_id: HostOperationId,
            request: PlatformHostRequest,
        ) -> Result<(), String> {
            if matches!(request, PlatformHostRequest::IndexedDb { .. }) {
                return self
                    .idb_sender
                    .send((operation_id, request))
                    .map_err(|_| "IndexedDB platform service is unavailable".to_string());
            }
            let context = self.context.clone();
            let sender = self.sender.clone();
            let is_http_start = matches!(request, PlatformHostRequest::HttpStart { .. });
            let cancellable_http_starts = self.cancellable_http_starts.clone();
            if is_http_start {
                cancellable_http_starts
                    .lock()
                    .map_err(|_| "HTTP cancellation registry is poisoned".to_string())?
                    .insert(operation_id);
            }
            std::thread::Builder::new()
                .name(format!("oryn-platform-{}", operation_id.0))
                .spawn(move || {
                    let result = execute_platform_request(&context, operation_id, request);
                    if is_http_start && let Ok(mut operations) = cancellable_http_starts.lock() {
                        operations.remove(&operation_id);
                    }
                    let _ = sender.send(PlatformHostCompletion {
                        operation_id,
                        result,
                    });
                })
                .map(|_| ())
                .map_err(|error| error.to_string())
        }

        fn cancel(&self, operation_id: HostOperationId) {
            if !self
                .cancellable_http_starts
                .lock()
                .is_ok_and(|operations| operations.contains(&operation_id))
            {
                return;
            }
            let context = self.context.clone();
            // Cancellation must not wait behind the operation being cancelled.
            // The multiplexed context transport carries this independent
            // request to the parent, where the broker owns the socket.
            let _ = std::thread::Builder::new()
                .name(format!("oryn-platform-cancel-{}", operation_id.0))
                .spawn(move || {
                    let _ = context
                        .client
                        .request(ContextServiceRequest::HttpRequestAbort {
                            page_id: context.page_id,
                            request_id: operation_id.0,
                        });
                });
        }

        fn drain_completions(&self, limit: usize) -> Vec<PlatformHostCompletion> {
            let Ok(receiver) = self.receiver.lock() else {
                return Vec::new();
            };
            (0..limit)
                .filter_map(|_| receiver.try_recv().ok())
                .collect()
        }
    }

    #[derive(Serialize)]
    struct FetchResult {
        url: String,
        status: u16,
        headers: Vec<(String, String)>,
        stream_id: Option<u64>,
        body_base64: String,
        body_chunks_base64: Vec<String>,
    }

    struct PendingFetch {
        context: v8::Global<v8::Context>,
        resolver: v8::Global<v8::PromiseResolver>,
    }

    struct FetchCompletion {
        operation_id: u64,
        result: Result<String, String>,
    }

    struct AsyncFetches {
        next_operation_id: u64,
        sender: mpsc::Sender<FetchCompletion>,
        receiver: mpsc::Receiver<FetchCompletion>,
        pending: BTreeMap<u64, PendingFetch>,
    }

    impl Default for AsyncFetches {
        fn default() -> Self {
            let (sender, receiver) = mpsc::channel();
            Self {
                next_operation_id: 1,
                sender,
                receiver,
                pending: BTreeMap::new(),
            }
        }
    }

    struct FetchRequest {
        input: String,
        method: String,
        body: Vec<u8>,
        headers: Vec<(String, String)>,
        credentials_mode: String,
    }

    struct DedicatedWorker {
        owner_identity: i32,
        context: v8::Global<v8::Context>,
        closed: bool,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum WorkerGlobalKind {
        Dedicated,
        Shared,
        Service,
    }

    impl WorkerGlobalKind {
        fn parse(value: &str) -> Result<Self, String> {
            match value {
                "" | "dedicatedworker" => Ok(Self::Dedicated),
                "sharedworker" => Ok(Self::Shared),
                "serviceworker" => Ok(Self::Service),
                _ => Err(format!("unsupported worker global {value:?}")),
            }
        }

        fn web_name(self) -> &'static str {
            match self {
                Self::Dedicated => "DedicatedWorkerGlobalScope",
                Self::Shared => "SharedWorkerGlobalScope",
                Self::Service => "ServiceWorkerGlobalScope",
            }
        }
    }

    struct DedicatedWorkers {
        next_id: u64,
        workers: BTreeMap<u64, DedicatedWorker>,
        outbound: BTreeMap<u64, Vec<String>>,
    }

    struct ScheduledTimer {
        context: v8::Global<v8::Context>,
        callback: v8::Global<v8::Function>,
        arguments: Vec<v8::Global<v8::Value>>,
        repeating: bool,
    }

    struct TimerRegistry {
        started: Instant,
        scheduler: PageScheduler<u64>,
        callbacks: BTreeMap<u64, ScheduledTimer>,
    }

    impl Default for TimerRegistry {
        fn default() -> Self {
            Self {
                started: Instant::now(),
                scheduler: PageScheduler::default(),
                callbacks: BTreeMap::new(),
            }
        }
    }

    impl Default for DedicatedWorkers {
        fn default() -> Self {
            Self {
                next_id: 1,
                workers: BTreeMap::new(),
                outbound: BTreeMap::new(),
            }
        }
    }

    macro_rules! install_host_functions {
        ($scope:expr, $context:expr) => {{
            let context_scope = &mut v8::ContextScope::new($scope, $context);
            if context_scope.get_slot::<FetchState>().is_some() {
                let function = v8::Function::new(context_scope, fetch_async_callback)
                    .expect("allocate native fetch callback");
                let name = v8::String::new(context_scope, "__oryn_fetch_async")
                    .expect("allocate native fetch name");
                $context
                    .global(context_scope)
                    .set(context_scope, name.into(), function.into());
            }
            #[cfg(unix)]
            if context_scope.get_slot::<ContextState>().is_some() {
                let function = v8::Function::new(context_scope, context_callback)
                    .expect("allocate native context callback");
                let name = v8::String::new(context_scope, "__oryn_context_call")
                    .expect("allocate native context callback name");
                $context
                    .global(context_scope)
                    .set(context_scope, name.into(), function.into());
                let function = v8::Function::new(context_scope, stream_read_async_callback)
                    .expect("allocate native stream callback");
                let name = v8::String::new(context_scope, "__oryn_stream_read_async")
                    .expect("allocate native stream callback name");
                $context
                    .global(context_scope)
                    .set(context_scope, name.into(), function.into());
            }
            let function = v8::Function::new(context_scope, crypto_callback)
                .expect("allocate native crypto callback");
            let name = v8::String::new(context_scope, "__oryn_crypto_sync")
                .expect("allocate native crypto callback name");
            $context
                .global(context_scope)
                .set(context_scope, name.into(), function.into());
            let function = v8::Function::new(context_scope, timer_schedule_callback)
                .expect("allocate native timer schedule callback");
            let name = v8::String::new(context_scope, "__oryn_timer_schedule")
                .expect("allocate native timer schedule name");
            $context
                .global(context_scope)
                .set(context_scope, name.into(), function.into());
            let function = v8::Function::new(context_scope, timer_cancel_callback)
                .expect("allocate native timer cancel callback");
            let name = v8::String::new(context_scope, "__oryn_timer_cancel")
                .expect("allocate native timer cancel name");
            $context
                .global(context_scope)
                .set(context_scope, name.into(), function.into());
            let function = v8::Function::new(context_scope, dedicated_worker_create_callback)
                .expect("allocate dedicated-worker create callback");
            let name = v8::String::new(context_scope, "__oryn_worker_create")
                .expect("allocate dedicated-worker create name");
            $context
                .global(context_scope)
                .set(context_scope, name.into(), function.into());
            let function = v8::Function::new(context_scope, dedicated_worker_post_callback)
                .expect("allocate dedicated-worker post callback");
            let name = v8::String::new(context_scope, "__oryn_worker_post")
                .expect("allocate dedicated-worker post name");
            $context
                .global(context_scope)
                .set(context_scope, name.into(), function.into());
            let function = v8::Function::new(context_scope, dedicated_worker_take_callback)
                .expect("allocate dedicated-worker take callback");
            let name = v8::String::new(context_scope, "__oryn_worker_take")
                .expect("allocate dedicated-worker take name");
            $context
                .global(context_scope)
                .set(context_scope, name.into(), function.into());
            let function = v8::Function::new(context_scope, dedicated_worker_terminate_callback)
                .expect("allocate dedicated-worker terminate callback");
            let name = v8::String::new(context_scope, "__oryn_worker_terminate")
                .expect("allocate dedicated-worker terminate name");
            $context
                .global(context_scope)
                .set(context_scope, name.into(), function.into());
            install_runtime_profile(context_scope);
            install_generated_binding_metadata(context_scope);
        }};
    }

    pub fn install_startup_snapshot_globals(
        scope: &mut v8::PinScope,
        context: v8::Local<v8::Context>,
    ) {
        install_host_functions!(scope, context);
    }

    fn timer_schedule_callback(
        scope: &mut v8::PinScope,
        args: v8::FunctionCallbackArguments,
        mut return_value: v8::ReturnValue<v8::Value>,
    ) {
        let result = (|| -> Result<u64, String> {
            let callback = v8::Local::<v8::Function>::try_from(args.get(0))
                .map_err(|_| "timer callback must be callable".to_string())?;
            let delay = args
                .get(1)
                .number_value(scope)
                .unwrap_or(0.0)
                .clamp(0.0, 2_147_483_647.0) as u64;
            let repeating = args.get(2).boolean_value(scope);
            let values = v8::Local::<v8::Array>::try_from(args.get(3))
                .map_err(|_| "timer arguments must be an array".to_string())?;
            let mut arguments = Vec::with_capacity(values.length() as usize);
            for index in 0..values.length() {
                let value = values
                    .get_index(scope, index)
                    .ok_or_else(|| "timer argument is unavailable".to_string())?;
                arguments.push(v8::Global::new(scope, value));
            }
            let context = v8::Global::new(scope, scope.get_current_context());
            let callback = v8::Global::new(scope, callback);
            let registry = scope
                .get_slot_mut::<TimerRegistry>()
                .ok_or_else(|| "timer scheduler is unavailable".to_string())?;
            let now_ms = registry.started.elapsed().as_millis() as u64;
            registry.scheduler.advance_to(now_ms);
            let delay = Duration::from_millis(if repeating { delay.max(1) } else { delay });
            let timer_id = if repeating {
                registry
                    .scheduler
                    .set_interval_with(delay, |timer_id| Task {
                        source: TaskSource::Timer,
                        realm: RealmId(0),
                        document_generation: 1,
                        action_id: None,
                        payload: timer_id,
                    })
            } else {
                registry.scheduler.set_timeout_with(delay, |timer_id| Task {
                    source: TaskSource::Timer,
                    realm: RealmId(0),
                    document_generation: 1,
                    action_id: None,
                    payload: timer_id,
                })
            };
            registry.callbacks.insert(
                timer_id,
                ScheduledTimer {
                    context,
                    callback,
                    arguments,
                    repeating,
                },
            );
            Ok(timer_id)
        })();
        match result {
            Ok(timer_id) => return_value.set(v8::Number::new(scope, timer_id as f64).into()),
            Err(error) => throw_type_error(scope, &error),
        }
    }

    fn timer_cancel_callback(
        scope: &mut v8::PinScope,
        args: v8::FunctionCallbackArguments,
        _return_value: v8::ReturnValue<v8::Value>,
    ) {
        let Some(timer_id) = args.get(0).integer_value(scope).filter(|id| *id > 0) else {
            return;
        };
        if let Some(registry) = scope.get_slot_mut::<TimerRegistry>() {
            let timer_id = timer_id as u64;
            registry.scheduler.cancel_timer(timer_id);
            registry.callbacks.remove(&timer_id);
        }
    }

    fn install_generated_binding_metadata(scope: &mut v8::ContextScope<v8::HandleScope>) {
        let interfaces = oryn_js::generated::GENERATED_INTERFACES
            .iter()
            .map(|interface| {
                serde_json::json!({
                    "name": interface.name,
                    "parent": interface.parent,
                    "members": interface.members.iter().map(|member| member.name).collect::<Vec<_>>(),
                })
            })
            .collect::<Vec<_>>();
        let encoded =
            serde_json::to_string(&interfaces).expect("generated binding metadata is serializable");
        let source = format!(
            "if(!Object.prototype.hasOwnProperty.call(globalThis,'__oryn_generated_bindings'))Object.defineProperty(globalThis,'__oryn_generated_bindings',{{value:Object.freeze({encoded}),enumerable:false}});"
        );
        let source = v8::String::new(scope, &source).expect("allocate generated binding metadata");
        let script = v8::Script::compile(scope, source, None)
            .expect("compile generated binding metadata installer");
        script
            .run(scope)
            .expect("install generated binding metadata");
    }

    fn install_runtime_profile(scope: &mut v8::ContextScope<v8::HandleScope>) {
        let profile = if cfg!(feature = "core-lite") {
            if option_env!("ORYN_ICU_DATA_SHA256").is_some() {
                "core-lite"
            } else {
                "core-lite-unattested-probe"
            }
        } else {
            "core-full"
        };
        let name = v8::String::new(scope, "__oryn_runtime_profile")
            .expect("allocate runtime profile name");
        let value = v8::String::new(scope, profile).expect("allocate runtime profile value");
        scope
            .get_current_context()
            .global(scope)
            .create_data_property(scope, name.into(), value.into());
    }

    impl RawV8Host {
        fn security_token_identity(origin: &str, realm: RealmId) -> String {
            // V8 uses context security-token identity for cross-context global
            // access. Same-origin frame realms must therefore share an exact
            // token, while opaque origins must never accidentally become
            // same-origin merely because their serialized origin is `null`.
            if origin == "null" {
                format!("oryn:opaque-realm:{}", realm.0)
            } else {
                format!("oryn:origin:{origin}")
            }
        }

        pub fn enter_realm(&self) {
            // SAFETY: ExecutableDocument is thread-affine and callers balance
            // every temporary child-realm entry with `exit_realm`.
            unsafe { self.isolate.enter() };
        }

        pub fn exit_realm(&self) {
            // SAFETY: this host is the currently entered child isolate and
            // V8 restores the previously entered page/frame isolate.
            unsafe { self.isolate.exit() };
        }

        /// Freeze an inactive history document. Page actors are thread-affine,
        /// so a retained isolate is always resumed on its creating thread.
        pub fn suspend_page_isolate(&mut self) {
            if self.isolate_entered {
                // SAFETY: callbacks and microtasks have reached a page
                // checkpoint and this host is the current entered isolate.
                unsafe { self.isolate.exit() };
                self.isolate_entered = false;
            }
        }

        pub fn resume_page_isolate(&mut self) {
            if !self.isolate_entered {
                // SAFETY: only one history document is resumed by the page
                // actor at a time, on the thread that created the isolate.
                unsafe { self.isolate.enter() };
                self.isolate_entered = true;
            }
        }

        pub fn new() -> Self {
            Self::create(None, None, None, 256 * 1024 * 1024)
        }

        pub fn new_with_network(broker: SharedNetworkBroker, base_url: Url) -> Self {
            let origin = base_url.origin().ascii_serialization();
            Self::create(
                Some(FetchState { broker, base_url }),
                None,
                Some(origin),
                256 * 1024 * 1024,
            )
        }

        pub fn new_with_network_and_heap_limit(
            broker: SharedNetworkBroker,
            base_url: Url,
            max_heap_bytes: usize,
        ) -> Self {
            let origin = base_url.origin().ascii_serialization();
            Self::create(
                Some(FetchState { broker, base_url }),
                None,
                Some(origin),
                max_heap_bytes,
            )
        }

        pub fn new_with_base_url_and_heap_limit(base_url: Url, max_heap_bytes: usize) -> Self {
            let origin = base_url.origin().ascii_serialization();
            Self::create(None, None, Some(origin), max_heap_bytes)
        }

        #[cfg(unix)]
        pub fn new_with_network_context_and_heap_limit(
            broker: Option<SharedNetworkBroker>,
            base_url: Option<Url>,
            context: SharedContextClient,
            page_id: PageId,
            max_heap_bytes: usize,
        ) -> Self {
            let origin = base_url
                .as_ref()
                .map(|url| url.origin().ascii_serialization());
            let fetch = broker
                .zip(base_url)
                .map(|(broker, base_url)| FetchState { broker, base_url });
            Self::create(
                fetch,
                Some(ContextState {
                    client: context,
                    page_id,
                }),
                origin,
                max_heap_bytes,
            )
        }

        pub fn new_with_heap_limit(max_heap_bytes: usize) -> Self {
            Self::create(None, None, None, max_heap_bytes)
        }

        #[cfg(test)]
        pub fn new_with_wall_limit(execution_wall_limit: Duration) -> Self {
            let mut host = Self::create(None, None, None, 256 * 1024 * 1024);
            host.execution_wall_limit = execution_wall_limit;
            host
        }

        fn create(
            fetch_state: Option<FetchState>,
            #[cfg(unix)] context_state: Option<ContextState>,
            #[cfg(not(unix))] _context_state: Option<()>,
            base_origin: Option<String>,
            max_heap_bytes: usize,
        ) -> Self {
            INITIALIZE_V8.call_once(|| {
                crate::initialize_v8_icu_data();
                let platform = v8::new_default_platform(0, false).make_shared();
                v8::V8::initialize_platform(platform);
                v8::V8::initialize();
                #[cfg(feature = "v8-sandbox")]
                assert_v8_heap_sandbox();
            });
            let params = v8::CreateParams::default()
                .heap_limits(0, max_heap_bytes)
                .external_references(startup_external_references());
            #[cfg(oryn_startup_snapshot)]
            let params = params.snapshot_blob(v8::StartupData::from(
                &include_bytes!(env!("ORYN_STARTUP_SNAPSHOT_PATH"))[..],
            ));
            let mut isolate = v8::Isolate::new(params);
            isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
            isolate.set_host_import_module_dynamically_callback(dynamic_import);
            isolate.set_host_initialize_import_meta_object_callback(initialize_import_meta);
            isolate.set_slot(PromiseRejections::default());
            isolate.set_slot(AsyncFetches::default());
            isolate.set_slot(DedicatedWorkers::default());
            isolate.set_slot(TimerRegistry::default());
            isolate.set_slot(ModuleNames::default());
            isolate.set_slot(ModuleCache::default());
            isolate.set_slot(ModuleResolutionFailures::default());
            isolate.set_promise_reject_callback(promise_reject_callback);
            if let Some(fetch_state) = fetch_state {
                isolate.set_slot(fetch_state);
            }
            #[cfg(unix)]
            if let Some(context_state) = context_state {
                isolate.set_slot(context_state);
            }
            let origin = base_origin
                .or_else(|| {
                    isolate
                        .get_slot::<FetchState>()
                        .map(|state| state.base_url.origin().ascii_serialization())
                })
                .unwrap_or_else(|| "null".into());
            let mut realms = RealmRegistry::default();
            let active_realm = realms.create(origin.clone(), 1, None);
            let (context, security_token) = {
                v8::scope!(let scope, &mut isolate);
                let context = v8::Context::new(scope, Default::default());
                let identity = Self::security_token_identity(&origin, active_realm);
                let token =
                    v8::String::new(scope, &identity).expect("allocate V8 context security token");
                context.set_security_token(token.into());
                install_host_functions!(scope, context);
                (
                    v8::Global::new(scope, context),
                    (identity, v8::Global::new(scope, token)),
                )
            };
            let mut contexts = BTreeMap::new();
            contexts.insert(active_realm, context);
            let mut security_tokens = BTreeMap::new();
            security_tokens.insert(security_token.0, security_token.1);
            Self {
                isolate,
                contexts,
                security_tokens,
                realms,
                active_realm,
                binding_states: BTreeMap::new(),
                trusted_types_config: None,
                execution_wall_limit: Duration::from_millis(9_500),
                isolate_entered: true,
            }
        }

        /// Install the generated browser interfaces and a Rust-owned document
        /// into the active realm. Calling this twice replaces the realm's
        /// binding state and therefore requires a new document generation.
        pub fn install_dom(
            &mut self,
            dom: SharedDom,
            document_generation: u64,
        ) -> Result<(), HostError> {
            #[cfg(unix)]
            let platform_context = self.isolate.get_slot::<ContextState>().cloned();
            let realm = self.active_realm;
            let context = self
                .contexts
                .get(&realm)
                .ok_or_else(|| HostError::Execution("active realm context is unavailable".into()))?
                .clone();
            v8::scope!(let handle_scope, &mut self.isolate);
            let context = v8::Local::new(handle_scope, &context);
            let scope = &mut v8::ContextScope::new(handle_scope, context);
            let state = install_generated_bindings(scope, context, dom, realm, document_generation)
                .map_err(|error| HostError::Execution(error.to_string()))?;
            #[cfg(unix)]
            if let Some(platform_context) = platform_context {
                state.set_platform_host(Rc::new(ContextPlatformHost::new(platform_context)));
            }
            if let Some((config, allow_eval)) = &self.trusted_types_config {
                state.configure_trusted_types(config.clone());
                context.set_allow_generation_from_strings(*allow_eval);
            }
            self.binding_states.insert(realm, state);
            Ok(())
        }

        pub fn set_execution_wall_limit(&mut self, limit: Duration) {
            self.execution_wall_limit = limit.max(Duration::from_millis(1));
        }

        pub fn install_dom_root(
            &mut self,
            dom: SharedDom,
            document_generation: u64,
            document: NodeId,
        ) -> Result<(), HostError> {
            #[cfg(unix)]
            let platform_context = self.isolate.get_slot::<ContextState>().cloned();
            let realm = self.active_realm;
            let context = self
                .contexts
                .get(&realm)
                .ok_or_else(|| HostError::Execution("active realm context is unavailable".into()))?
                .clone();
            v8::scope!(let handle_scope, &mut self.isolate);
            let context = v8::Local::new(handle_scope, &context);
            let scope = &mut v8::ContextScope::new(handle_scope, context);
            let state = install_generated_bindings_with_root(
                scope,
                context,
                dom,
                realm,
                document_generation,
                document,
            )
            .map_err(|error| HostError::Execution(error.to_string()))?;
            #[cfg(unix)]
            if let Some(platform_context) = platform_context {
                state.set_platform_host(Rc::new(ContextPlatformHost::new(platform_context)));
            }
            if let Some((config, allow_eval)) = &self.trusted_types_config {
                state.configure_trusted_types(config.clone());
                context.set_allow_generation_from_strings(*allow_eval);
            }
            self.binding_states.insert(realm, state);
            Ok(())
        }

        pub fn set_frame_document(
            &mut self,
            frame: NodeId,
            document: NodeId,
        ) -> Result<(), HostError> {
            self.binding_states
                .get(&self.active_realm)
                .ok_or_else(|| HostError::Execution("active Rust DOM is unavailable".into()))?
                .set_frame_document(frame, document);
            Ok(())
        }

        pub fn set_frame_realm(
            &mut self,
            frame: NodeId,
            document: NodeId,
            realm: RealmId,
        ) -> Result<(), HostError> {
            self.link_frame_realm(frame, Some(document), realm)
        }

        pub fn set_opaque_frame_realm(
            &mut self,
            frame: NodeId,
            realm: RealmId,
        ) -> Result<(), HostError> {
            self.link_frame_realm(frame, None, realm)
        }

        /// Install the stable WindowProxy that parser-inserted frames expose
        /// immediately, before their navigation has produced a child realm.
        /// Messages sent through it are queued by the bootstrap and delivered
        /// when `link_frame_realm` binds the committed realm.
        pub fn ensure_provisional_frame_window(&mut self, frame: NodeId) -> Result<(), HostError> {
            let state = self
                .binding_states
                .get(&self.active_realm)
                .cloned()
                .ok_or_else(|| HostError::Execution("active Rust DOM is unavailable".into()))?;
            if state.frame_window(frame).is_some() {
                return Ok(());
            }
            let context = self
                .contexts
                .get(&self.active_realm)
                .ok_or_else(|| HostError::Execution("active realm context is unavailable".into()))?
                .clone();
            v8::scope!(let handle_scope, &mut self.isolate);
            let context = v8::Local::new(handle_scope, &context);
            let scope = &mut v8::ContextScope::new(handle_scope, context);
            let global = context.global(scope);
            let key = v8::String::new(scope, "__oryn_make_provisional_window_proxy")
                .ok_or_else(|| HostError::Execution("allocate provisional proxy key".into()))?;
            let factory = global
                .get(scope, key.into())
                .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
                .ok_or_else(|| {
                    HostError::Execution("provisional proxy factory is unavailable".into())
                })?;
            let proxy = factory
                .call(scope, global.into(), &[])
                .and_then(|value| value.to_object(scope))
                .ok_or_else(|| HostError::Execution("create provisional WindowProxy".into()))?;
            state.set_frame_window(frame, v8::Global::new(scope, proxy));
            Ok(())
        }

        fn link_frame_realm(
            &mut self,
            frame: NodeId,
            document: Option<NodeId>,
            realm: RealmId,
        ) -> Result<(), HostError> {
            let child_context = self
                .contexts
                .get(&realm)
                .ok_or_else(|| HostError::Execution("frame realm is unavailable".into()))?
                .clone();
            let parent_context = self
                .contexts
                .get(&self.active_realm)
                .ok_or_else(|| HostError::Execution("parent realm is unavailable".into()))?
                .clone();
            let active = self
                .binding_states
                .get(&self.active_realm)
                .cloned()
                .ok_or_else(|| HostError::Execution("active Rust DOM is unavailable".into()))?;
            let parent_origin = self
                .realms
                .get(self.active_realm)
                .map(|descriptor| descriptor.origin.clone())
                .ok_or_else(|| {
                    HostError::Execution("parent realm descriptor is unavailable".into())
                })?;
            let child_origin = self
                .realms
                .get(realm)
                .map(|descriptor| descriptor.origin.clone())
                .ok_or_else(|| {
                    HostError::Execution("frame realm descriptor is unavailable".into())
                })?;
            let same_origin = document.is_some() && parent_origin == child_origin;
            v8::scope!(let handle_scope, &mut self.isolate);
            let parent_context_local = v8::Local::new(handle_scope, &parent_context);
            let child_context_local = v8::Local::new(handle_scope, &child_context);
            let parent_window =
                v8::Global::new(handle_scope, parent_context_local.global(handle_scope));
            let child_window =
                v8::Global::new(handle_scope, child_context_local.global(handle_scope));
            let parent_deliver = {
                let parent_context = v8::Local::new(handle_scope, &parent_context);
                let scope = &mut v8::ContextScope::new(handle_scope, parent_context);
                let window = v8::Local::new(scope, &parent_window);
                let key = v8::String::new(scope, "__oryn_deliver_message")
                    .ok_or_else(|| HostError::Execution("allocate parent deliver key".into()))?;
                let function = window
                    .get(scope, key.into())
                    .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
                    .ok_or_else(|| {
                        HostError::Execution("parent message delivery is unavailable".into())
                    })?;
                v8::Global::new(scope, function)
            };
            let child_deliver = {
                let child_context = v8::Local::new(handle_scope, &child_context);
                let scope = &mut v8::ContextScope::new(handle_scope, child_context);
                let window = v8::Local::new(scope, &child_window);
                let key = v8::String::new(scope, "__oryn_deliver_message")
                    .ok_or_else(|| HostError::Execution("allocate frame deliver key".into()))?;
                let function = window
                    .get(scope, key.into())
                    .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
                    .ok_or_else(|| {
                        HostError::Execution("frame message delivery is unavailable".into())
                    })?;
                v8::Global::new(scope, function)
            };
            let parent_opener = {
                let parent_context = v8::Local::new(handle_scope, &parent_context);
                let scope = &mut v8::ContextScope::new(handle_scope, parent_context);
                let window = v8::Local::new(scope, &parent_window);
                let key = v8::String::new(scope, "opener")
                    .ok_or_else(|| HostError::Execution("allocate opener key".into()))?;
                let value = window
                    .get(scope, key.into())
                    .unwrap_or_else(|| v8::undefined(scope).into());
                v8::Global::new(scope, value)
            };
            let child_proxy = {
                let parent_context = v8::Local::new(handle_scope, &parent_context);
                let scope = &mut v8::ContextScope::new(handle_scope, parent_context);
                let window = v8::Local::new(scope, &parent_window);
                let factory_key =
                    v8::String::new(scope, "__oryn_make_window_proxy").ok_or_else(|| {
                        HostError::Execution("allocate frame proxy factory key".into())
                    })?;
                let factory = window
                    .get(scope, factory_key.into())
                    .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
                    .ok_or_else(|| {
                        HostError::Execution("frame proxy factory is unavailable".into())
                    })?;
                let deliver = v8::Local::new(scope, &child_deliver);
                let target_origin = v8::String::new(scope, &child_origin)
                    .ok_or_else(|| HostError::Execution("allocate child origin".into()))?;
                let source_origin = v8::String::new(scope, &parent_origin)
                    .ok_or_else(|| HostError::Execution("allocate parent origin".into()))?;
                let target: v8::Local<v8::Value> = if same_origin {
                    v8::Local::new(scope, &child_window).into()
                } else {
                    v8::null(scope).into()
                };
                let proxy = factory
                    .call(
                        scope,
                        window.into(),
                        &[
                            deliver.into(),
                            target_origin.into(),
                            source_origin.into(),
                            target,
                        ],
                    )
                    .and_then(|value| value.to_object(scope))
                    .ok_or_else(|| HostError::Execution("create child WindowProxy".into()))?;
                v8::Global::new(scope, proxy)
            };
            let parent_proxy = {
                let child_context = v8::Local::new(handle_scope, &child_context);
                let scope = &mut v8::ContextScope::new(handle_scope, child_context);
                let window = v8::Local::new(scope, &child_window);
                let factory_key =
                    v8::String::new(scope, "__oryn_make_window_proxy").ok_or_else(|| {
                        HostError::Execution("allocate parent proxy factory key".into())
                    })?;
                let factory = window
                    .get(scope, factory_key.into())
                    .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
                    .ok_or_else(|| {
                        HostError::Execution("parent proxy factory is unavailable".into())
                    })?;
                let deliver = v8::Local::new(scope, &parent_deliver);
                let target_origin = v8::String::new(scope, &parent_origin)
                    .ok_or_else(|| HostError::Execution("allocate parent origin".into()))?;
                let source_origin = v8::String::new(scope, &child_origin)
                    .ok_or_else(|| HostError::Execution("allocate child origin".into()))?;
                let target: v8::Local<v8::Value> = if same_origin {
                    v8::Local::new(scope, &parent_window).into()
                } else {
                    v8::null(scope).into()
                };
                let proxy = factory
                    .call(
                        scope,
                        window.into(),
                        &[
                            deliver.into(),
                            target_origin.into(),
                            source_origin.into(),
                            target,
                        ],
                    )
                    .and_then(|value| value.to_object(scope))
                    .ok_or_else(|| HostError::Execution("create parent WindowProxy".into()))?;
                v8::Global::new(scope, proxy)
            };
            let public_child_proxy = if let Some(provisional) = active.frame_window(frame) {
                let parent_context = v8::Local::new(handle_scope, &parent_context);
                let scope = &mut v8::ContextScope::new(handle_scope, parent_context);
                let provisional = v8::Local::new(scope, &provisional);
                let bind_key = v8::String::new(scope, "__bind")
                    .ok_or_else(|| HostError::Execution("allocate provisional bind key".into()))?;
                let bind = provisional
                    .get(scope, bind_key.into())
                    .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
                    .ok_or_else(|| {
                        HostError::Execution("provisional WindowProxy cannot bind".into())
                    })?;
                let committed = v8::Local::new(scope, &child_proxy);
                bind.call(scope, provisional.into(), &[committed.into()])
                    .ok_or_else(|| HostError::Execution("bind provisional WindowProxy".into()))?;
                v8::Global::new(scope, provisional)
            } else {
                child_proxy.clone()
            };
            {
                let parent_context = v8::Local::new(handle_scope, &parent_context);
                let scope = &mut v8::ContextScope::new(handle_scope, parent_context);
                let child_proxy = v8::Local::new(scope, &child_proxy);
                let public_child_proxy = v8::Local::new(scope, &public_child_proxy);
                let parent_proxy = v8::Local::new(scope, &parent_proxy);
                let source_key = v8::String::new(scope, "__source")
                    .ok_or_else(|| HostError::Execution("allocate child proxy source".into()))?;
                child_proxy.set(scope, source_key.into(), parent_proxy.into());
                public_child_proxy.set(scope, source_key.into(), parent_proxy.into());
            }
            {
                let child_context = v8::Local::new(handle_scope, &child_context);
                let scope = &mut v8::ContextScope::new(handle_scope, child_context);
                let child_window = v8::Local::new(scope, &child_window);
                let child_proxy = v8::Local::new(scope, &public_child_proxy);
                let parent_proxy = v8::Local::new(scope, &parent_proxy);
                let source_key = v8::String::new(scope, "__source")
                    .ok_or_else(|| HostError::Execution("allocate parent proxy source".into()))?;
                parent_proxy.set(scope, source_key.into(), child_proxy.into());
                let opener_key = v8::String::new(scope, "opener")
                    .ok_or_else(|| HostError::Execution("allocate parent opener".into()))?;
                let opener = v8::Local::new(scope, &parent_opener);
                parent_proxy.set(scope, opener_key.into(), opener);
                // A top-level browsing context is its own `parent` and `top`.
                // These properties are part of the cross-origin WindowProxy
                // surface too: testharness walks `self -> parent -> top`
                // without being allowed to inspect the parent's DOM. Leaving
                // them absent turns the second walk step into `undefined` and
                // aborts every cross-origin child harness during bootstrap.
                if !same_origin {
                    for name in ["parent", "top"] {
                        let key = v8::String::new(scope, name).ok_or_else(|| {
                            HostError::Execution(format!("allocate parent proxy {name}"))
                        })?;
                        parent_proxy.set(scope, key.into(), parent_proxy.into());
                    }
                }
                for name in ["parent", "top"] {
                    let key = v8::String::new(scope, name)
                        .ok_or_else(|| HostError::Execution(format!("allocate frame {name}")))?;
                    child_window.set(scope, key.into(), parent_proxy.into());
                }
            }
            if let Some(document) = document {
                active.set_frame_document(frame, document);
            }
            active.set_frame_window(frame, public_child_proxy);
            Ok(())
        }

        pub fn set_current_script(&mut self, anchor: Option<NodeId>) -> Result<(), HostError> {
            self.binding_states
                .get(&self.active_realm)
                .ok_or_else(|| HostError::Execution("active Rust DOM is unavailable".into()))?
                .set_current_script(anchor);
            Ok(())
        }

        pub fn set_parser_input_active(&mut self, active: bool) -> Result<(), HostError> {
            self.binding_states
                .get(&self.active_realm)
                .ok_or_else(|| HostError::Execution("active Rust DOM is unavailable".into()))?
                .set_parser_input_active(active);
            Ok(())
        }

        pub fn take_parser_input_commands(
            &mut self,
        ) -> Result<Vec<oryn_js::v8_bindings::ParserInputCommand>, HostError> {
            Ok(self
                .binding_states
                .get(&self.active_realm)
                .ok_or_else(|| HostError::Execution("active Rust DOM is unavailable".into()))?
                .take_parser_input_commands())
        }

        pub fn take_history_traversal(&self) -> Result<Option<i64>, HostError> {
            self.binding_states
                .get(&self.active_realm)
                .ok_or_else(|| HostError::Execution("active Rust DOM is unavailable".into()))
                .map(|state| state.take_history_traversal())
        }

        pub fn configure_trusted_types(
            &mut self,
            config: oryn_js::trusted_types::TrustedTypesConfig,
            allow_string_code_generation: bool,
        ) -> Result<(), HostError> {
            let realm = self.active_realm;
            self.trusted_types_config = Some((config.clone(), allow_string_code_generation));
            self.binding_states
                .get(&realm)
                .ok_or_else(|| HostError::Execution("active Rust DOM is unavailable".into()))?
                .configure_trusted_types(config);
            let context = self
                .contexts
                .get(&realm)
                .ok_or_else(|| HostError::Execution("active realm context is unavailable".into()))?
                .clone();
            v8::scope!(let handle_scope, &mut self.isolate);
            let context = v8::Local::new(handle_scope, &context);
            context.set_allow_generation_from_strings(allow_string_code_generation);
            Ok(())
        }

        pub fn active_dom(&self) -> Option<SharedDom> {
            self.binding_states
                .get(&self.active_realm)
                .map(|state| state.dom())
        }

        pub fn take_pending_dynamic_scripts(&self) -> Result<Vec<NodeId>, HostError> {
            self.binding_states
                .get(&self.active_realm)
                .ok_or_else(|| HostError::Execution("active Rust DOM is unavailable".into()))
                .map(|state| state.take_pending_dynamic_scripts())
        }

        pub fn take_pending_dynamic_frames(&self) -> Result<Vec<NodeId>, HostError> {
            self.binding_states
                .get(&self.active_realm)
                .ok_or_else(|| HostError::Execution("active Rust DOM is unavailable".into()))
                .map(|state| state.take_pending_dynamic_frames())
        }

        pub fn install_layout(&mut self, layout: BindingLayout) -> Result<(), HostError> {
            self.binding_states
                .get(&self.active_realm)
                .ok_or_else(|| HostError::Execution("active Rust DOM is unavailable".into()))?
                .set_layout(layout);
            Ok(())
        }

        pub fn install_layout_provider(
            &mut self,
            provider: Rc<dyn BindingLayoutProvider>,
        ) -> Result<(), HostError> {
            self.binding_states
                .get(&self.active_realm)
                .ok_or_else(|| HostError::Execution("active Rust DOM is unavailable".into()))?
                .set_layout_provider(provider);
            Ok(())
        }

        pub fn refresh_named_window_properties(&mut self) -> Result<(), HostError> {
            let realm = self.active_realm;
            let state = self
                .binding_states
                .get(&realm)
                .cloned()
                .ok_or_else(|| HostError::Execution("active Rust DOM is unavailable".into()))?;
            let context = self
                .contexts
                .get(&realm)
                .cloned()
                .ok_or_else(|| HostError::Execution("active realm is unavailable".into()))?;
            let document = state.document_root();
            v8::scope!(let handle_scope, &mut self.isolate);
            let context = v8::Local::new(handle_scope, &context);
            let scope = &mut v8::ContextScope::new(handle_scope, context);
            publish_named_window_properties(scope, context, &state, document)
                .map_err(|error| HostError::Execution(error.to_string()))
        }

        /// Invoke generated JavaScript behavior with a Rust-owned node exposed
        /// through a short-lived, non-enumerable task global.
        pub fn execute_with_node(
            &mut self,
            node: NodeId,
            global_name: &str,
            source: &str,
        ) -> Result<(), HostError> {
            let realm = self.active_realm;
            let state = self
                .binding_states
                .get(&realm)
                .cloned()
                .ok_or_else(|| HostError::Execution("active Rust DOM is unavailable".into()))?;
            let context = self
                .contexts
                .get(&realm)
                .cloned()
                .ok_or_else(|| HostError::Execution("active realm is unavailable".into()))?;
            {
                v8::scope!(let handle_scope, &mut self.isolate);
                let context = v8::Local::new(handle_scope, &context);
                let scope = &mut v8::ContextScope::new(handle_scope, context);
                let wrapper = wrap_node(scope, &state, node)
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                let name = v8::String::new(scope, global_name).ok_or_else(|| {
                    HostError::Execution("action global allocation failed".into())
                })?;
                context.global(scope).define_own_property(
                    scope,
                    name.into(),
                    wrapper.into(),
                    v8::PropertyAttribute::DONT_ENUM,
                );
            }
            let result = self.execute(source);
            let cleanup = format!("delete globalThis[{global_name:?}]");
            let cleanup_result = self.execute(&cleanup);
            result.and(cleanup_result)
        }

        /// Evaluate a boolean expression with a Rust-owned node exposed through
        /// the same short-lived task global used by generated action bindings.
        pub fn evaluate_bool_with_node(
            &mut self,
            node: NodeId,
            global_name: &str,
            source: &str,
        ) -> Result<bool, HostError> {
            let realm = self.active_realm;
            let state = self
                .binding_states
                .get(&realm)
                .cloned()
                .ok_or_else(|| HostError::Execution("active Rust DOM is unavailable".into()))?;
            let context = self
                .contexts
                .get(&realm)
                .cloned()
                .ok_or_else(|| HostError::Execution("active realm is unavailable".into()))?;
            {
                v8::scope!(let handle_scope, &mut self.isolate);
                let context = v8::Local::new(handle_scope, &context);
                let scope = &mut v8::ContextScope::new(handle_scope, context);
                let wrapper = wrap_node(scope, &state, node)
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                let name = v8::String::new(scope, global_name).ok_or_else(|| {
                    HostError::Execution("action global allocation failed".into())
                })?;
                context.global(scope).define_own_property(
                    scope,
                    name.into(),
                    wrapper.into(),
                    v8::PropertyAttribute::DONT_ENUM,
                );
            }
            let result = self.evaluate_string(source);
            let cleanup = format!("delete globalThis[{global_name:?}]");
            let cleanup_result = self.execute(&cleanup);
            cleanup_result?;
            Ok(result? == "true")
        }

        pub fn process_binding_releases(&mut self) -> Result<usize, HostError> {
            let released_connected_roots = {
                v8::scope!(let handle_scope, &mut self.isolate);
                let mut released = 0;
                for (realm, state) in &self.binding_states {
                    let context = v8::Local::new(
                        handle_scope,
                        self.contexts
                            .get(realm)
                            .expect("binding realm context exists"),
                    );
                    let scope = &mut v8::ContextScope::new(handle_scope, context);
                    released += state.refresh_connected_wrapper_roots(scope);
                }
                released
            };
            if released_connected_roots > 0 {
                self.isolate.low_memory_notification();
            }
            self.binding_states
                .values()
                .try_fold(0_usize, |total, state| {
                    state
                        .process_weak_releases()
                        .map(|released| total + released)
                        .map_err(|error| HostError::Execution(error.to_string()))
                })
        }

        /// Deliver Rust DOM mutation journals after V8's microtask checkpoint.
        /// Observer callbacks may mutate again, so repeat to a bounded stable
        /// point before style/semantic publication.
        pub fn run_dom_observer_checkpoint(&mut self) -> Result<usize, HostError> {
            let mut delivered = 0_usize;
            for _ in 0..1024 {
                let Some(dom) = self.active_dom() else {
                    return Ok(delivered);
                };
                let mutations = dom.borrow_mut().drain_mutations();
                if mutations.is_empty() {
                    let realms = self.binding_states.keys().copied().collect::<Vec<_>>();
                    for realm in realms {
                        let Some(state) = self.binding_states.get(&realm).cloned() else {
                            continue;
                        };
                        let Some(context) = self.contexts.get(&realm).cloned() else {
                            continue;
                        };
                        v8::scope!(let handle_scope, &mut self.isolate);
                        let context = v8::Local::new(handle_scope, &context);
                        let scope = &mut v8::ContextScope::new(handle_scope, context);
                        delivered += usize::from(
                            deliver_selection_change(scope, &state)
                                .map_err(|error| HostError::Execution(error.to_string()))?,
                        );
                    }
                    return Ok(delivered);
                }
                let realms = self.binding_states.keys().copied().collect::<Vec<_>>();
                for realm in realms {
                    let Some(state) = self.binding_states.get(&realm).cloned() else {
                        continue;
                    };
                    let Some(context) = self.contexts.get(&realm).cloned() else {
                        continue;
                    };
                    v8::scope!(let handle_scope, &mut self.isolate);
                    let context = v8::Local::new(handle_scope, &context);
                    let scope = &mut v8::ContextScope::new(handle_scope, context);
                    delivered += deliver_mutation_observers(scope, &state, &mutations)
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                    delivered += usize::from(
                        deliver_selection_change(scope, &state)
                            .map_err(|error| HostError::Execution(error.to_string()))?,
                    );
                    scope.perform_microtask_checkpoint();
                }
                if let Some(dom) = self.active_dom() {
                    dom.borrow_mut()
                        .release_mutation_roots(&mutations)
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                }
            }
            Err(HostError::LimitExceeded {
                resource: "mutation observer checkpoint iterations".into(),
            })
        }

        pub fn next_timer_delay(&mut self) -> Option<Duration> {
            let registry = self.isolate.get_slot_mut::<TimerRegistry>()?;
            registry
                .scheduler
                .advance_to(registry.started.elapsed().as_millis() as u64);
            registry
                .scheduler
                .next_timer_delay_ms()
                .map(Duration::from_millis)
        }

        pub fn drain_scheduled_timers(&mut self, wait: bool) -> Result<usize, HostError> {
            if wait
                && let Some(delay) = self.next_timer_delay()
                && !delay.is_zero()
                && delay <= self.execution_wall_limit
            {
                std::thread::sleep(delay);
            }
            let now_ms = self
                .isolate
                .get_slot::<TimerRegistry>()
                .map(|registry| registry.started.elapsed().as_millis() as u64)
                .unwrap_or(0);
            if let Some(registry) = self.isolate.get_slot_mut::<TimerRegistry>() {
                registry.scheduler.advance_to(now_ms);
            }
            let mut delivered = 0_usize;
            while delivered < 1024 {
                let mut timer_id = None;
                if let Some(registry) = self.isolate.get_slot_mut::<TimerRegistry>() {
                    registry
                        .scheduler
                        .run_checkpoint(|task| timer_id = Some(task.payload));
                }
                let Some(timer_id) = timer_id else { break };
                let timer = self
                    .isolate
                    .get_slot::<TimerRegistry>()
                    .and_then(|registry| registry.callbacks.get(&timer_id))
                    .map(|timer| {
                        (
                            timer.context.clone(),
                            timer.callback.clone(),
                            timer.arguments.clone(),
                            timer.repeating,
                        )
                    });
                let Some((context, callback, arguments, repeating)) = timer else {
                    continue;
                };
                {
                    let _deadline =
                        ExecutionDeadline::arm(&self.isolate, self.execution_wall_limit);
                    v8::scope!(let handle_scope, &mut self.isolate);
                    let context = v8::Local::new(handle_scope, &context);
                    let scope = &mut v8::ContextScope::new(handle_scope, context);
                    v8::tc_scope!(let scope, scope);
                    let callback = v8::Local::new(scope, &callback);
                    let arguments = arguments
                        .iter()
                        .map(|value| v8::Local::new(scope, value))
                        .collect::<Vec<_>>();
                    let receiver = v8::undefined(scope).into();
                    if callback.call(scope, receiver, &arguments).is_none() {
                        let detail = scope
                            .exception()
                            .and_then(|value| value.to_string(scope))
                            .map(|value| value.to_rust_string_lossy(scope))
                            .unwrap_or_else(|| "timer callback failed".into());
                        return Err(HostError::Execution(detail));
                    }
                    scope.perform_microtask_checkpoint();
                }
                if !repeating && let Some(registry) = self.isolate.get_slot_mut::<TimerRegistry>() {
                    registry.callbacks.remove(&timer_id);
                }
                delivered += 1;
            }
            Ok(delivered)
        }

        /// Deliver parent-broker completions outside the originating V8
        /// callback. Waiting is bounded by the page execution wall limit and
        /// all ready completions run a real V8 microtask checkpoint.
        pub fn drain_async_fetches(&mut self, wait: bool) -> Result<usize, HostError> {
            let deadline = std::time::Instant::now() + self.execution_wall_limit;
            let mut delivered = 0_usize;
            loop {
                let completion = {
                    let fetches = self
                        .isolate
                        .get_slot_mut::<AsyncFetches>()
                        .expect("asynchronous fetch registry installed");
                    if fetches.pending.is_empty() {
                        break;
                    }
                    if wait {
                        let remaining =
                            deadline.saturating_duration_since(std::time::Instant::now());
                        if remaining.is_zero() {
                            return Err(HostError::LimitExceeded {
                                resource: "asynchronous fetch wall time".into(),
                            });
                        }
                        match fetches
                            .receiver
                            .recv_timeout(remaining.min(Duration::from_millis(50)))
                        {
                            Ok(completion) => Some(completion),
                            Err(mpsc::RecvTimeoutError::Timeout) => continue,
                            Err(mpsc::RecvTimeoutError::Disconnected) => {
                                return Err(HostError::Execution(
                                    "asynchronous fetch completion channel closed".into(),
                                ));
                            }
                        }
                    } else {
                        match fetches.receiver.try_recv() {
                            Ok(completion) => Some(completion),
                            Err(mpsc::TryRecvError::Empty) => None,
                            Err(mpsc::TryRecvError::Disconnected) => {
                                return Err(HostError::Execution(
                                    "asynchronous fetch completion channel closed".into(),
                                ));
                            }
                        }
                    }
                };
                let Some(completion) = completion else {
                    break;
                };
                let pending = self
                    .isolate
                    .get_slot_mut::<AsyncFetches>()
                    .expect("asynchronous fetch registry installed")
                    .pending
                    .remove(&completion.operation_id)
                    .ok_or_else(|| {
                        HostError::Execution(format!(
                            "unmatched asynchronous fetch completion {}",
                            completion.operation_id
                        ))
                    })?;
                v8::scope!(let handle_scope, &mut self.isolate);
                let context = v8::Local::new(handle_scope, &pending.context);
                let scope = &mut v8::ContextScope::new(handle_scope, context);
                let resolver = v8::Local::new(scope, &pending.resolver);
                match completion.result {
                    Ok(result) => {
                        let value = v8::String::new(scope, &result).ok_or_else(|| {
                            HostError::Execution("fetch result allocation failed".into())
                        })?;
                        resolver.resolve(scope, value.into());
                    }
                    Err(error) => {
                        let message = v8::String::new(scope, &error).ok_or_else(|| {
                            HostError::Execution("fetch error allocation failed".into())
                        })?;
                        let exception = v8::Exception::error(scope, message);
                        resolver.reject(scope, exception);
                    }
                }
                scope.perform_microtask_checkpoint();
                delivered += 1;
            }
            delivered += self.drain_generated_platform_operations(wait)?;
            Ok(delivered)
        }

        fn drain_generated_platform_operations(&mut self, wait: bool) -> Result<usize, HostError> {
            // A transaction keep-alive loop is legitimate browser activity:
            // every success handler may enqueue the next request forever.
            // Bound one scheduler checkpoint so that such a chain yields to
            // other page tasks instead of being misreported as a wall-time
            // violation. Operations that make no progress still wait until
            // the normal execution deadline and fail closed.
            const MAX_COMPLETIONS_PER_CHECKPOINT: usize = 1_024;
            let deadline = Instant::now() + self.execution_wall_limit;
            let mut delivered = 0;
            loop {
                let mut pass = 0;
                let realms = self.binding_states.keys().copied().collect::<Vec<_>>();
                for realm in realms {
                    let Some(state) = self.binding_states.get(&realm).cloned() else {
                        continue;
                    };
                    let Some(context) = self.contexts.get(&realm).cloned() else {
                        continue;
                    };
                    v8::scope!(let handle_scope, &mut self.isolate);
                    let context = v8::Local::new(handle_scope, &context);
                    let scope = &mut v8::ContextScope::new(handle_scope, context);
                    pass += state
                        .drain_platform_completions(scope, 32)
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                    if pass > 0 {
                        scope.perform_microtask_checkpoint();
                        state
                            .finish_platform_task_checkpoint()
                            .map_err(|error| HostError::Execution(error.to_string()))?;
                    }
                }
                delivered += pass;
                // Timers are peer page tasks. Run ready timers between
                // platform-completion turns so a self-sustaining request
                // chain cannot starve cancellation, transaction release, or
                // other zero-delay work.
                delivered += self.drain_scheduled_timers(false)?;
                if delivered >= MAX_COMPLETIONS_PER_CHECKPOINT {
                    break;
                }
                let pending = self
                    .binding_states
                    .values()
                    .any(|state| state.has_pending_platform_work());
                if !wait || !pending {
                    break;
                }
                if Instant::now() >= deadline {
                    return Err(HostError::LimitExceeded {
                        resource: "generated platform operation wall time".into(),
                    });
                }
                if pass == 0 {
                    // Parent-service database operations are normally tiny.
                    // A millisecond polling floor compounds into seconds for
                    // large cursor/partition suites, even though the ordered
                    // service has already made the next completion runnable.
                    // Yield briefly without busy-spinning or delaying network
                    // operations by an entire scheduler quantum.
                    std::thread::sleep(Duration::from_micros(50));
                }
            }
            Ok(delivered)
        }

        pub fn has_pending_async_fetches(&self) -> bool {
            self.isolate
                .get_slot::<AsyncFetches>()
                .is_some_and(|fetches| !fetches.pending.is_empty())
        }

        pub fn has_pending_page_work(&mut self) -> bool {
            self.has_pending_async_fetches()
                || self.next_timer_delay().is_some()
                || self
                    .binding_states
                    .values()
                    .any(|state| state.has_pending_platform_work())
        }

        /// Create another global realm in this page's isolate. DOM wrappers are
        /// still installed separately; this method only establishes the V8
        /// context and its immutable security identity.
        pub fn create_realm(
            &mut self,
            origin: impl Into<String>,
            document_generation: u64,
            frame_id: Option<u64>,
        ) -> RealmId {
            let origin = origin.into();
            let realm = self
                .realms
                .create(origin.clone(), document_generation, frame_id);
            let identity = Self::security_token_identity(&origin, realm);
            let existing_token = self.security_tokens.get(&identity).cloned();
            let context = {
                v8::scope!(let scope, &mut self.isolate);
                let context = v8::Context::new(scope, Default::default());
                let token = existing_token
                    .as_ref()
                    .map(|token| v8::Local::new(scope, token))
                    .unwrap_or_else(|| {
                        v8::String::new(scope, &identity)
                            .expect("allocate V8 context security token")
                    });
                context.set_security_token(token.into());
                if existing_token.is_none() {
                    self.security_tokens
                        .insert(identity, v8::Global::new(scope, token));
                }
                install_host_functions!(scope, context);
                v8::Global::new(scope, context)
            };
            self.contexts.insert(realm, context);
            realm
        }

        pub fn create_realm_with_dom(
            &mut self,
            origin: impl Into<String>,
            document_generation: u64,
            frame_id: Option<u64>,
            dom: Rc<RefCell<oryn_dom::DomRuntime>>,
        ) -> Result<RealmId, HostError> {
            let previous = self.active_realm;
            let realm = self.create_realm(origin, document_generation, frame_id);
            self.activate_realm(realm)?;
            let installed = self.install_dom(dom, document_generation);
            self.activate_realm(previous)?;
            installed.map(|()| realm)
        }

        pub fn create_realm_with_dom_root(
            &mut self,
            origin: impl Into<String>,
            document_generation: u64,
            frame_id: Option<u64>,
            dom: Rc<RefCell<oryn_dom::DomRuntime>>,
            document: NodeId,
        ) -> Result<RealmId, HostError> {
            let previous = self.active_realm;
            let realm = self.create_realm(origin, document_generation, frame_id);
            self.activate_realm(realm)?;
            let installed = self.install_dom_root(dom, document_generation, document);
            self.activate_realm(previous)?;
            installed.map(|()| realm)
        }

        pub fn activate_realm(&mut self, realm: RealmId) -> Result<(), HostError> {
            if !self.contexts.contains_key(&realm) {
                return Err(HostError::Execution(format!(
                    "unknown JavaScript realm {}",
                    realm.0
                )));
            }
            self.active_realm = realm;
            Ok(())
        }

        pub fn active_realm(&self) -> RealmId {
            self.active_realm
        }

        pub fn realm_descriptor(&self, realm: RealmId) -> Option<&RealmDescriptor> {
            self.realms.get(realm)
        }

        /// Replace or remove a frame realm after navigation. The top-level
        /// realm cannot be removed because callers always need an active
        /// execution context.
        pub fn remove_realm(&mut self, realm: RealmId) -> Result<RealmDescriptor, HostError> {
            if self.contexts.len() == 1 || realm == RealmId(0) {
                return Err(HostError::Execution(
                    "the top-level JavaScript realm cannot be removed".into(),
                ));
            }
            self.contexts.remove(&realm).ok_or_else(|| {
                HostError::Execution(format!("unknown JavaScript realm {}", realm.0))
            })?;
            self.binding_states.remove(&realm);
            if self.active_realm == realm {
                self.active_realm = RealmId(0);
            }
            self.realms.remove(realm).ok_or_else(|| {
                HostError::Execution(format!("missing descriptor for realm {}", realm.0))
            })
        }

        pub fn execute_module(
            &mut self,
            resource_name: &str,
            source: &str,
            dependencies: &BTreeMap<String, String>,
        ) -> Result<(), HostError> {
            let _deadline = ExecutionDeadline::arm(&self.isolate, self.execution_wall_limit);
            self.isolate.set_slot(ModuleSources(dependencies.clone()));
            v8::scope!(let handle_scope, &mut self.isolate);
            let context = v8::Local::new(
                handle_scope,
                self.contexts
                    .get(&self.active_realm)
                    .expect("active realm context exists"),
            );
            let scope = &mut v8::ContextScope::new(handle_scope, context);
            let cached = scope
                .get_slot::<ModuleCache>()
                .and_then(|cache| cache.0.get(resource_name))
                .cloned();
            let module = if let Some(cached) = cached {
                v8::Local::new(scope, cached)
            } else {
                let mut source = module_source(scope, resource_name, source)?;
                let module = v8::script_compiler::compile_module(scope, &mut source)
                    .ok_or_else(|| HostError::Execution("module compilation failed".into()))?;
                if let Some(names) = scope.get_slot_mut::<ModuleNames>() {
                    names
                        .0
                        .insert(module.get_identity_hash().get(), resource_name.into());
                }
                let global = v8::Global::new(scope, module);
                scope
                    .get_slot_mut::<ModuleCache>()
                    .expect("module cache installed")
                    .0
                    .insert(resource_name.into(), global);
                module
            };
            if module.get_status() == v8::ModuleStatus::Uninstantiated {
                let instantiated = module
                    .instantiate_module(scope, resolve_module)
                    .ok_or_else(|| HostError::Execution("module linking failed".into()))?;
                if !instantiated {
                    return Err(HostError::Execution("module linking failed".into()));
                }
            }
            if matches!(module.get_status(), v8::ModuleStatus::Evaluated) {
                return Ok(());
            }
            let Some(evaluation) = module.evaluate(scope) else {
                if _deadline.expired() || scope.is_execution_terminating() {
                    scope.cancel_terminate_execution();
                    return Err(HostError::LimitExceeded {
                        resource: "wall time".into(),
                    });
                }
                return Err(HostError::Execution("module evaluation failed".into()));
            };
            scope.perform_microtask_checkpoint();
            if let Ok(promise) = v8::Local::<v8::Promise>::try_from(evaluation)
                && promise.state() == v8::PromiseState::Rejected
            {
                return Err(HostError::Execution(format!(
                    "module evaluation rejected: {}",
                    promise.result(scope).to_rust_string_lossy(scope)
                )));
            }
            if _deadline.expired() || scope.is_execution_terminating() {
                scope.cancel_terminate_execution();
                return Err(HostError::LimitExceeded {
                    resource: "wall time".into(),
                });
            }
            Ok(())
        }

        pub fn set_module_sources(&mut self, sources: &BTreeMap<String, String>) {
            self.isolate.set_slot(ModuleSources(sources.clone()));
        }

        pub fn take_unhandled_promise_rejections(&mut self) -> Vec<String> {
            self.isolate
                .get_slot_mut::<PromiseRejections>()
                .map(|rejections| std::mem::take(&mut rejections.0).into_values().collect())
                .unwrap_or_default()
        }

        pub fn take_module_resolution_failures(&mut self) -> Vec<String> {
            self.isolate
                .get_slot_mut::<ModuleResolutionFailures>()
                .map(|failures| std::mem::take(&mut failures.0))
                .unwrap_or_default()
        }
    }

    extern "C" fn promise_reject_callback(message: v8::PromiseRejectMessage) {
        v8::callback_scope!(unsafe scope, &message);
        let promise = message.get_promise();
        let identity = promise.get_identity_hash().get();
        match message.get_event() {
            v8::PromiseRejectEvent::PromiseRejectWithNoHandler => {
                let detail = message
                    .get_value()
                    .map(|value| value.to_rust_string_lossy(scope))
                    .unwrap_or_else(|| "promise rejected without a value".into());
                if let Some(rejections) = scope.get_slot_mut::<PromiseRejections>() {
                    rejections.0.insert(identity, detail);
                }
            }
            v8::PromiseRejectEvent::PromiseHandlerAddedAfterReject => {
                if let Some(rejections) = scope.get_slot_mut::<PromiseRejections>() {
                    rejections.0.remove(&identity);
                }
            }
            _ => {}
        }
    }

    extern "C" fn initialize_import_meta(
        context: v8::Local<v8::Context>,
        module: v8::Local<v8::Module>,
        meta: v8::Local<v8::Object>,
    ) {
        v8::callback_scope!(unsafe scope, context);
        v8::scope!(let scope, scope);
        let resource_name = scope
            .get_slot::<ModuleNames>()
            .and_then(|names| names.0.get(&module.get_identity_hash().get()))
            .cloned()
            .unwrap_or_else(|| "oryn:inline-module".into());
        let url = Url::parse(&resource_name)
            .ok()
            .or_else(|| {
                scope
                    .get_slot::<FetchState>()
                    .and_then(|fetch| fetch.base_url.join(&resource_name).ok())
            })
            .map(|value| value.to_string())
            .unwrap_or(resource_name);
        if let (Some(key), Some(value)) =
            (v8::String::new(scope, "url"), v8::String::new(scope, &url))
        {
            meta.create_data_property(scope, key.into(), value.into());
        }
    }

    fn dynamic_import<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        _host_defined_options: v8::Local<'s, v8::Data>,
        resource_name: v8::Local<'s, v8::Value>,
        specifier: v8::Local<'s, v8::String>,
        _import_attributes: v8::Local<'s, v8::FixedArray>,
    ) -> Option<v8::Local<'s, v8::Promise>> {
        let resolver = v8::PromiseResolver::new(scope)?;
        let specifier = specifier.to_rust_string_lossy(scope);
        let resource_name = resource_name.to_rust_string_lossy(scope);
        let result = (|| -> Result<v8::Local<'s, v8::Value>, String> {
            let mut sources = scope
                .get_slot::<ModuleSources>()
                .ok_or_else(|| "module source map is unavailable".to_string())?
                .0
                .clone();
            let resolved = resolve_module_name(&resource_name, &specifier, &sources)
                .or_else(|| fetch_module_source(scope, &resource_name, &specifier))
                .ok_or_else(|| format!("module source {specifier:?} is unavailable"))?;
            if let Some(updated) = scope.get_slot::<ModuleSources>() {
                sources = updated.0.clone();
            }
            let cached = scope
                .get_slot::<ModuleCache>()
                .and_then(|cache| cache.0.get(&resolved))
                .cloned();
            let module = if let Some(cached) = cached {
                v8::Local::new(scope, cached)
            } else {
                let source = sources
                    .get(&resolved)
                    .ok_or_else(|| format!("resolved module source {resolved:?} is unavailable"))?;
                let mut source = module_source(scope, &resolved, source)
                    .map_err(|error| format!("module source allocation failed: {error}"))?;
                let module = v8::script_compiler::compile_module(scope, &mut source)
                    .ok_or_else(|| format!("module {resolved:?} compilation failed"))?;
                if let Some(names) = scope.get_slot_mut::<ModuleNames>() {
                    names
                        .0
                        .insert(module.get_identity_hash().get(), resolved.clone());
                }
                let global = v8::Global::new(scope, module);
                scope
                    .get_slot_mut::<ModuleCache>()
                    .ok_or_else(|| "module cache is unavailable".to_string())?
                    .0
                    .insert(resolved.clone(), global);
                module
            };
            if module.get_status() == v8::ModuleStatus::Uninstantiated
                && !module
                    .instantiate_module(scope, resolve_module)
                    .ok_or_else(|| format!("module {resolved:?} linking failed"))?
            {
                return Err(format!("module {resolved:?} linking failed"));
            }
            if module.get_status() == v8::ModuleStatus::Instantiated {
                module
                    .evaluate(scope)
                    .ok_or_else(|| format!("module {resolved:?} evaluation failed"))?;
            }
            Ok(module.get_module_namespace())
        })();
        match result {
            Ok(namespace) => {
                resolver.resolve(scope, namespace);
            }
            Err(error) => {
                let detail = v8::String::new(
                    scope,
                    &format!("dynamic module {specifier:?} failed: {error}"),
                )?;
                resolver.reject(scope, detail.into());
            }
        }
        Some(resolver.get_promise(scope))
    }

    fn resolve_module_name(
        resource_name: &str,
        specifier: &str,
        sources: &BTreeMap<String, String>,
    ) -> Option<String> {
        if let Some(target) = sources.get(&format!("oryn:module-alias:{specifier}"))
            && sources.contains_key(target)
        {
            return Some(target.clone());
        }
        if sources.contains_key(specifier) {
            return Some(specifier.to_string());
        }
        if let Ok(base) = Url::parse(resource_name)
            && let Ok(joined) = base.join(specifier)
            && sources.contains_key(joined.as_str())
        {
            return Some(joined.to_string());
        }
        let base = std::path::Path::new(resource_name)
            .parent()
            .unwrap_or_else(|| std::path::Path::new(""));
        let joined = base.join(specifier).to_string_lossy().to_string();
        if sources.contains_key(&joined) {
            return Some(joined);
        }
        let normalized = normalize_module_path(&joined);
        sources
            .keys()
            .find(|candidate| normalize_module_path(candidate) == normalized)
            .cloned()
    }

    fn normalize_module_path(value: &str) -> String {
        use std::path::Component;
        let mut parts = Vec::new();
        for component in std::path::Path::new(value).components() {
            match component {
                Component::ParentDir => {
                    parts.pop();
                }
                Component::Normal(value) => parts.push(value.to_string_lossy().to_string()),
                Component::RootDir => parts.clear(),
                Component::CurDir | Component::Prefix(_) => {}
            }
        }
        parts.join("/")
    }

    const DEDICATED_WORKER_BOOTSTRAP: &str = r#"
        (() => {
          const listeners = new Map();
          class WorkerEvent {
            constructor(type, init = {}) { this.type = String(type); this.defaultPrevented = false; Object.assign(this, init); }
            preventDefault() { this.defaultPrevented = true; }
          }
          class WorkerEventTarget {
            addEventListener(type, callback) {
              const key = String(type), list = this.__listeners?.get(key) || [];
              list.push(callback); (this.__listeners || (this.__listeners = new Map())).set(key, list);
            }
            removeEventListener(type, callback) {
              const key = String(type);
              this.__listeners?.set(key, (this.__listeners.get(key) || []).filter(item => item !== callback));
            }
            dispatchEvent(event) {
              for (const callback of this.__listeners?.get(event.type) || []) callback.call(this, event);
              const handler = this['on' + event.type];
              if (typeof handler === 'function') handler.call(this, event);
              return !event.defaultPrevented;
            }
          }
          class DedicatedWorkerGlobalScope extends WorkerEventTarget {}
          class SharedWorkerGlobalScope extends WorkerEventTarget {}
          class ServiceWorkerGlobalScope extends WorkerEventTarget {}
          const cloneJson = value => {
            const encoded = JSON.stringify(value);
            if (encoded === undefined) {
              const error = new TypeError('dedicated-worker messages must be structured-cloneable JSON values');
              error.name = 'DataCloneError';
              throw error;
            }
            return encoded;
          };
          globalThis.self = globalThis;
          globalThis.Event = WorkerEvent;
          globalThis.MessageEvent = WorkerEvent;
          globalThis.EventTarget = WorkerEventTarget;
          globalThis.queueMicrotask = callback => Promise.resolve().then(callback);
          globalThis.setTimeout = (callback, delay = 0, ...args) => __oryn_timer_schedule(callback, delay, false, args);
          globalThis.setInterval = (callback, delay = 0, ...args) => __oryn_timer_schedule(callback, delay, true, args);
          globalThis.clearTimeout = id => __oryn_timer_cancel(Number(id));
          globalThis.clearInterval = globalThis.clearTimeout;
          globalThis.DedicatedWorkerGlobalScope = DedicatedWorkerGlobalScope;
          globalThis.SharedWorkerGlobalScope = SharedWorkerGlobalScope;
          globalThis.ServiceWorkerGlobalScope = ServiceWorkerGlobalScope;
          const Scope = globalThis[globalThis.__oryn_worker_global_name] || DedicatedWorkerGlobalScope;
          Object.setPrototypeOf(globalThis, Scope.prototype);
          Object.defineProperty(globalThis, Symbol.toStringTag, {value:globalThis.__oryn_worker_global_name});
          globalThis.onmessage = null;
          globalThis.onerror = null;
          globalThis.addEventListener = (type, callback) => {
            const key = String(type), list = listeners.get(key) || [];
            list.push(callback); listeners.set(key, list);
          };
          globalThis.removeEventListener = (type, callback) => {
            const key = String(type);
            listeners.set(key, (listeners.get(key) || []).filter(item => item !== callback));
          };
          globalThis.dispatchEvent = event => {
            for (const callback of listeners.get(event.type) || []) callback.call(globalThis, event);
            const handler = globalThis['on' + event.type];
            if (typeof handler === 'function') handler.call(globalThis, event);
            return true;
          };
          globalThis.postMessage = value => __oryn_worker_emit(cloneJson(value));
          globalThis.close = () => __oryn_worker_close();
          const sharedPort = new WorkerEventTarget();
          sharedPort.postMessage = value => __oryn_worker_emit(cloneJson(value));
          sharedPort.start = () => {};
          sharedPort.close = () => __oryn_worker_close();
          globalThis.__oryn_dispatch_message = encoded => {
            const event = new MessageEvent('message', { data: JSON.parse(encoded) });
            if (globalThis.__oryn_worker_global_name === 'SharedWorkerGlobalScope') sharedPort.dispatchEvent(event);
            else dispatchEvent(event);
          };
          globalThis.__oryn_connect_shared = () => dispatchEvent(new MessageEvent('connect', {ports:[sharedPort]}));
        })();
    "#;

    // Parent-owned IndexedDB is available in every worker realm. The worker
    // holds only request/cursor objects; all durable state and quota decisions
    // remain behind the typed context-service operation.
    const WORKER_INDEXED_DB_BOOTSTRAP: &str = r#"
      (() => {
        if (typeof globalThis.__oryn_context_call !== 'function') return;
        const call = command => {
          const outer = JSON.parse(__oryn_context_call(JSON.stringify({operation:'storage',request:{operation:'indexed_db',origin:location.origin,command}})));
          if (outer.result !== 'storage' || outer.response.result !== 'indexed_db') throw new Error('invalid IndexedDB response');
          return outer.response.response;
        };
        const clone = value => value === undefined ? undefined : JSON.parse(JSON.stringify(value));
        class Request extends EventTarget {
          constructor(){super();this.result=undefined;this.error=null;this.transaction=null;this.onsuccess=null;this.onerror=null;this.onupgradeneeded=null;this.onblocked=null}
          fire(type,extra={}){const event=new Event(type,{target:this,...extra});event.target=this;this.dispatchEvent(event)}
          success(value){queueMicrotask(()=>{this.result=value;this.fire('success')})}
          failure(error){queueMicrotask(()=>{this.error=error;this.fire('error')})}
        }
        class Cursor {
          constructor(request,store,records,keyOnly=false,index=0){this.request=request;this.store=store;this.records=records;this.keyOnly=keyOnly;this.index=index;this.sync()}
          sync(){const record=this.records[this.index];this.key=record?.key;this.primaryKey=record?.primary_key;this.value=this.keyOnly?undefined:clone(record?.value)}
          continue(key){let next=this.index+1;if(key!==undefined)next=this.records.findIndex((record,index)=>index>this.index&&indexedDB.cmp(record.key,key)>=0);this.index=next<0?this.records.length:next;queueMicrotask(()=>{if(this.index>=this.records.length)this.request.success(null);else{this.sync();this.request.success(this)}})}
          advance(count){this.index+=Math.max(1,Number(count)||1)-1;this.continue()}
          delete(){return this.store.delete(this.primaryKey)}
          update(value){return this.store.put(value,this.primaryKey)}
        }
        class Index {
          constructor(store,name){this.objectStore=store;this.name=String(name)}
          records(query,direction='next'){return call({command:'cursor',database:this.objectStore.transaction.db.name,store:this.objectStore.name,index:this.name,query:query??null,reverse:String(direction).startsWith('prev')}).records}
          request(transform){const request=new Request();try{request.success(transform())}catch(error){request.failure(error)}return request}
          get(query){return this.request(()=>clone(this.records(query)[0]?.value))}
          getKey(query){return this.request(()=>clone(this.records(query)[0]?.primary_key))}
          getAll(query,count){return this.request(()=>this.records(query).slice(0,count??Infinity).map(record=>clone(record.value)))}
          getAllKeys(query,count){return this.request(()=>this.records(query).slice(0,count??Infinity).map(record=>clone(record.primary_key)))}
          count(query){return this.request(()=>this.records(query).length)}
          openCursor(query,direction='next'){return this.objectStore.cursor(query,direction,this.name,false)}
          openKeyCursor(query,direction='next'){return this.objectStore.cursor(query,direction,this.name,true)}
        }
        class ObjectStore {
          constructor(transaction,name){this.transaction=transaction;this.name=String(name)}
          request(command,transform=value=>value){const request=new Request();try{request.success(transform(call(command)))}catch(error){request.failure(error)}return request}
          put(value,key){return this.request({command:'batch',database:this.transaction.db.name,writes:[{store:this.name,key:key??null,value:clone(value),add_only:false}]},response=>response.key)}
          add(value,key){return this.request({command:'batch',database:this.transaction.db.name,writes:[{store:this.name,key:key??null,value:clone(value),add_only:true}]},response=>response.key)}
          get(key){return this.request({command:'get',database:this.transaction.db.name,store:this.name,key},response=>clone(response.value))}
          getAll(query,count){if(query===undefined)return this.request({command:'get_all',database:this.transaction.db.name,store:this.name},response=>response.values.slice(0,count??Infinity).map(clone));return this.request({command:'cursor',database:this.transaction.db.name,store:this.name,index:null,query,reverse:false},response=>response.records.slice(0,count??Infinity).map(record=>clone(record.value)))}
          getAllKeys(query,count){return this.request({command:'cursor',database:this.transaction.db.name,store:this.name,index:null,query:query??null,reverse:false},response=>response.records.slice(0,count??Infinity).map(record=>clone(record.primary_key)))}
          delete(key){return this.request({command:'delete',database:this.transaction.db.name,store:this.name,key})}
          clear(){return this.request({command:'clear',database:this.transaction.db.name,store:this.name})}
          count(query){return this.request(query===undefined?{command:'count',database:this.transaction.db.name,store:this.name}:{command:'cursor',database:this.transaction.db.name,store:this.name,index:null,query,reverse:false},response=>response.count??response.records.length)}
          cursor(query,direction,index,keyOnly){const request=new Request();request.store=this;try{const records=call({command:'cursor',database:this.transaction.db.name,store:this.name,index,query:query??null,reverse:String(direction).startsWith('prev')}).records;queueMicrotask(()=>request.success(records.length?new Cursor(request,this,records,keyOnly):null))}catch(error){request.failure(error)}return request}
          openCursor(query,direction='next'){return this.cursor(query,direction,null,false)}
          openKeyCursor(query,direction='next'){return this.cursor(query,direction,null,true)}
          createIndex(name,keyPath,options={}){call({command:'create_index',database:this.transaction.db.name,store:this.name,definition:{name:String(name),key_path:String(keyPath),unique:!!options.unique}});return new Index(this,name)}
          index(name){return new Index(this,name)}
        }
        class Transaction extends EventTarget {
          constructor(db,names,mode='readonly'){super();this.db=db;this.mode=mode;this.objectStoreNames=Array.isArray(names)?names.map(String):[String(names)];this.oncomplete=null;this.onabort=null;this.onerror=null;queueMicrotask(()=>this.dispatchEvent(new Event('complete',{target:this})))}
          objectStore(name){if(!this.objectStoreNames.includes(String(name)))throw new Error('NotFoundError');return new ObjectStore(this,name)}
          abort(){this.dispatchEvent(new Event('abort',{target:this}))}
        }
        class Database extends EventTarget {
          constructor(name,version,stores=[]){super();this.name=String(name);this.version=version;this.objectStoreNames=stores}
          createObjectStore(name,options={}){const response=call({command:'create_object_store',database:this.name,definition:{name:String(name),key_path:options.keyPath??null,auto_increment:!!options.autoIncrement}});this.objectStoreNames=response.object_stores;return new ObjectStore(this.__upgradeTransaction||(this.__upgradeTransaction=new Transaction(this,[name],'versionchange')),name)}
          transaction(names,mode='readonly'){return new Transaction(this,names,mode)}
          close(){}
        }
        const compare=(left,right)=>{if(typeof left==='number'&&typeof right==='number')return Math.sign(left-right);if(typeof left==='string'&&typeof right==='string')return left<right?-1:left>right?1:0;const a=JSON.stringify(left),b=JSON.stringify(right);return a<b?-1:a>b?1:0};
        globalThis.IDBRequest=Request;globalThis.IDBOpenDBRequest=Request;globalThis.IDBCursor=Cursor;globalThis.IDBIndex=Index;globalThis.IDBObjectStore=ObjectStore;globalThis.IDBTransaction=Transaction;globalThis.IDBDatabase=Database;
        globalThis.indexedDB={cmp:compare,open(name,version=0){const request=new Request();try{const response=call({command:'open',name:String(name),version:Number(version)||0});const db=new Database(name,response.version,response.object_stores);const transaction=new Transaction(db,response.object_stores,'versionchange');db.__upgradeTransaction=transaction;request.transaction=transaction;queueMicrotask(()=>{request.result=db;if(response.upgrade_needed)request.fire('upgradeneeded',{oldVersion:response.old_version,newVersion:response.version});request.fire('success')})}catch(error){request.failure(error)}return request},deleteDatabase(name){const request=new Request();try{call({command:'delete_database',name:String(name)});request.success(undefined)}catch(error){request.failure(error)}return request}};
      })();
    "#;

    fn worker_id_argument(
        scope: &mut v8::PinScope,
        args: &v8::FunctionCallbackArguments,
    ) -> Result<u64, String> {
        args.get(0)
            .integer_value(scope)
            .filter(|value| *value > 0)
            .map(|value| value as u64)
            .ok_or_else(|| "invalid dedicated-worker id".into())
    }

    fn current_global_identity(scope: &mut v8::PinScope) -> i32 {
        scope
            .get_current_context()
            .global(scope)
            .get_identity_hash()
            .get()
    }

    fn throw_type_error(scope: &mut v8::PinScope, detail: &str) {
        if let Some(message) = v8::String::new(scope, detail) {
            let exception = v8::Exception::type_error(scope, message);
            scope.throw_exception(exception);
        }
    }

    fn dedicated_worker_create_callback(
        scope: &mut v8::PinScope,
        args: v8::FunctionCallbackArguments,
        mut return_value: v8::ReturnValue<v8::Value>,
    ) {
        let created = (|| -> Result<u64, String> {
            let source = args
                .get(0)
                .to_string(scope)
                .map(|value| value.to_rust_string_lossy(scope))
                .ok_or_else(|| "worker source is not string-convertible".to_string())?;
            let resource_name = args
                .get(1)
                .to_string(scope)
                .map(|value| value.to_rust_string_lossy(scope))
                .unwrap_or_else(|| "oryn:dedicated-worker".into());
            let module = args.get(2).boolean_value(scope);
            let global_kind = WorkerGlobalKind::parse(
                &args
                    .get(3)
                    .to_string(scope)
                    .map(|value| value.to_rust_string_lossy(scope))
                    .unwrap_or_default(),
            )?;
            let owner_identity = current_global_identity(scope);
            let worker_id = {
                let registry = scope
                    .get_slot_mut::<DedicatedWorkers>()
                    .ok_or_else(|| "dedicated-worker registry unavailable".to_string())?;
                if registry.workers.len() >= 32 {
                    return Err("dedicated-worker limit exceeded".into());
                }
                let worker_id = registry.next_id;
                registry.next_id = worker_id.saturating_add(1);
                worker_id
            };
            let context = v8::Context::new(scope, Default::default());
            install_host_functions!(scope, context);
            let context_global = v8::Global::new(scope, context);
            scope
                .get_slot_mut::<DedicatedWorkers>()
                .expect("dedicated-worker registry installed")
                .workers
                .insert(
                    worker_id,
                    DedicatedWorker {
                        owner_identity,
                        context: context_global,
                        closed: false,
                    },
                );

            let executed = (|| -> Result<(), String> {
                let worker_scope = &mut v8::ContextScope::new(scope, context);
                let id_name = v8::String::new(worker_scope, "__oryn_worker_id")
                    .ok_or_else(|| "worker id allocation failed".to_string())?;
                worker_scope.get_current_context().global(worker_scope).set(
                    worker_scope,
                    id_name.into(),
                    v8::Number::new(worker_scope, worker_id as f64).into(),
                );
                let emit = v8::Function::new(worker_scope, dedicated_worker_emit_callback)
                    .ok_or_else(|| "worker emit callback allocation failed".to_string())?;
                let name = v8::String::new(worker_scope, "__oryn_worker_emit")
                    .ok_or_else(|| "worker emit name allocation failed".to_string())?;
                worker_scope.get_current_context().global(worker_scope).set(
                    worker_scope,
                    name.into(),
                    emit.into(),
                );
                let close = v8::Function::new(worker_scope, dedicated_worker_close_callback)
                    .ok_or_else(|| "worker close callback allocation failed".to_string())?;
                let name = v8::String::new(worker_scope, "__oryn_worker_close")
                    .ok_or_else(|| "worker close name allocation failed".to_string())?;
                worker_scope.get_current_context().global(worker_scope).set(
                    worker_scope,
                    name.into(),
                    close.into(),
                );
                let parsed_url = Url::parse(&resource_name).ok();
                let url = serde_json::to_string(&resource_name).map_err(|e| e.to_string())?;
                let origin = serde_json::to_string(
                    &parsed_url
                        .as_ref()
                        .map(|url| url.origin().ascii_serialization())
                        .unwrap_or_else(|| "null".into()),
                )
                .map_err(|error| error.to_string())?;
                let pathname = serde_json::to_string(parsed_url.as_ref().map_or("", Url::path))
                    .map_err(|error| error.to_string())?;
                let global_name = serde_json::to_string(global_kind.web_name())
                    .map_err(|error| error.to_string())?;
                let bootstrap = format!(
                    "globalThis.location=Object.freeze({{href:{url},origin:{origin},pathname:{pathname}}});globalThis.__oryn_worker_global_name={global_name};{}{}",
                    DEDICATED_WORKER_BOOTSTRAP, WORKER_INDEXED_DB_BOOTSTRAP
                );
                let bootstrap = v8::String::new(worker_scope, &bootstrap)
                    .ok_or_else(|| "worker bootstrap allocation failed".to_string())?;
                let bootstrap = v8::Script::compile(worker_scope, bootstrap, None)
                    .ok_or_else(|| "worker bootstrap compilation failed".to_string())?;
                bootstrap
                    .run(worker_scope)
                    .ok_or_else(|| "worker bootstrap execution failed".to_string())?;
                if module {
                    let mut source = module_source(worker_scope, &resource_name, &source)
                        .map_err(|error| error.to_string())?;
                    let module = v8::script_compiler::compile_module(worker_scope, &mut source)
                        .ok_or_else(|| "worker module compilation failed".to_string())?;
                    if !module
                        .instantiate_module(worker_scope, resolve_module)
                        .ok_or_else(|| "worker module linking failed".to_string())?
                    {
                        return Err("worker module linking failed".into());
                    }
                    module
                        .evaluate(worker_scope)
                        .ok_or_else(|| "worker module evaluation failed".to_string())?;
                } else {
                    let source = v8::String::new(worker_scope, &source)
                        .ok_or_else(|| "worker source allocation failed".to_string())?;
                    let script = v8::Script::compile(worker_scope, source, None)
                        .ok_or_else(|| "worker source compilation failed".to_string())?;
                    script
                        .run(worker_scope)
                        .ok_or_else(|| "worker source execution failed".to_string())?;
                }
                if global_kind == WorkerGlobalKind::Shared {
                    let connect = v8::String::new(worker_scope, "__oryn_connect_shared()")
                        .ok_or_else(|| "shared-worker connect allocation failed".to_string())?;
                    let connect = v8::Script::compile(worker_scope, connect, None)
                        .ok_or_else(|| "shared-worker connect compilation failed".to_string())?;
                    connect
                        .run(worker_scope)
                        .ok_or_else(|| "shared-worker connect dispatch failed".to_string())?;
                }
                worker_scope.perform_microtask_checkpoint();
                Ok(())
            })();
            if let Err(error) = executed {
                if let Some(registry) = scope.get_slot_mut::<DedicatedWorkers>() {
                    registry.workers.remove(&worker_id);
                    registry.outbound.remove(&worker_id);
                }
                return Err(error);
            }
            Ok(worker_id)
        })();
        match created {
            Ok(worker_id) => return_value.set(v8::Number::new(scope, worker_id as f64).into()),
            Err(error) => throw_type_error(scope, &error),
        }
    }

    fn dedicated_worker_post_callback(
        scope: &mut v8::PinScope,
        args: v8::FunctionCallbackArguments,
        mut return_value: v8::ReturnValue<v8::Value>,
    ) {
        let result = (|| -> Result<bool, String> {
            let worker_id = worker_id_argument(scope, &args)?;
            let encoded = args
                .get(1)
                .to_string(scope)
                .map(|value| value.to_rust_string_lossy(scope))
                .ok_or_else(|| "worker message is not string-convertible".to_string())?;
            let owner_identity = current_global_identity(scope);
            let context = {
                let registry = scope
                    .get_slot::<DedicatedWorkers>()
                    .ok_or_else(|| "dedicated-worker registry unavailable".to_string())?;
                let worker = registry
                    .workers
                    .get(&worker_id)
                    .ok_or_else(|| "dedicated worker is unavailable".to_string())?;
                if worker.owner_identity != owner_identity {
                    return Err("dedicated worker belongs to another realm".into());
                }
                if worker.closed {
                    return Ok(false);
                }
                worker.context.clone()
            };
            let context = v8::Local::new(scope, &context);
            let worker_scope = &mut v8::ContextScope::new(scope, context);
            let encoded = serde_json::to_string(&encoded).map_err(|error| error.to_string())?;
            let source =
                v8::String::new(worker_scope, &format!("__oryn_dispatch_message({encoded})"))
                    .ok_or_else(|| "worker message allocation failed".to_string())?;
            let script = v8::Script::compile(worker_scope, source, None)
                .ok_or_else(|| "worker message compilation failed".to_string())?;
            script
                .run(worker_scope)
                .ok_or_else(|| "worker message dispatch failed".to_string())?;
            worker_scope.perform_microtask_checkpoint();
            Ok(true)
        })();
        match result {
            Ok(value) => return_value.set(v8::Boolean::new(scope, value).into()),
            Err(error) => throw_type_error(scope, &error),
        }
    }

    fn dedicated_worker_take_callback(
        scope: &mut v8::PinScope,
        args: v8::FunctionCallbackArguments,
        mut return_value: v8::ReturnValue<v8::Value>,
    ) {
        let result = (|| -> Result<String, String> {
            let worker_id = worker_id_argument(scope, &args)?;
            let owner_identity = current_global_identity(scope);
            let registry = scope
                .get_slot_mut::<DedicatedWorkers>()
                .ok_or_else(|| "dedicated-worker registry unavailable".to_string())?;
            let worker = registry
                .workers
                .get(&worker_id)
                .ok_or_else(|| "dedicated worker is unavailable".to_string())?;
            if worker.owner_identity != owner_identity {
                return Err("dedicated worker belongs to another realm".into());
            }
            serde_json::to_string(&registry.outbound.remove(&worker_id).unwrap_or_default())
                .map_err(|error| error.to_string())
        })();
        match result {
            Ok(encoded) => {
                if let Some(encoded) = v8::String::new(scope, &encoded) {
                    return_value.set(encoded.into());
                }
            }
            Err(error) => throw_type_error(scope, &error),
        }
    }

    fn dedicated_worker_terminate_callback(
        scope: &mut v8::PinScope,
        args: v8::FunctionCallbackArguments,
        _return_value: v8::ReturnValue<v8::Value>,
    ) {
        let result = (|| -> Result<(), String> {
            let worker_id = worker_id_argument(scope, &args)?;
            let owner_identity = current_global_identity(scope);
            let registry = scope
                .get_slot_mut::<DedicatedWorkers>()
                .ok_or_else(|| "dedicated-worker registry unavailable".to_string())?;
            if registry
                .workers
                .get(&worker_id)
                .is_some_and(|worker| worker.owner_identity != owner_identity)
            {
                return Err("dedicated worker belongs to another realm".into());
            }
            registry.workers.remove(&worker_id);
            registry.outbound.remove(&worker_id);
            Ok(())
        })();
        if let Err(error) = result {
            throw_type_error(scope, &error);
        }
    }

    fn dedicated_worker_emit_callback(
        scope: &mut v8::PinScope,
        args: v8::FunctionCallbackArguments,
        _return_value: v8::ReturnValue<v8::Value>,
    ) {
        let result = (|| -> Result<(), String> {
            let global = scope.get_current_context().global(scope);
            let name = v8::String::new(scope, "__oryn_worker_id")
                .ok_or_else(|| "worker id name allocation failed".to_string())?;
            let worker_id = global
                .get(scope, name.into())
                .and_then(|value| value.integer_value(scope))
                .filter(|value| *value > 0)
                .map(|value| value as u64)
                .ok_or_else(|| "worker realm has no identity".to_string())?;
            let encoded = args
                .get(0)
                .to_string(scope)
                .map(|value| value.to_rust_string_lossy(scope))
                .ok_or_else(|| "worker message is not string-convertible".to_string())?;
            let registry = scope
                .get_slot_mut::<DedicatedWorkers>()
                .ok_or_else(|| "dedicated-worker registry unavailable".to_string())?;
            if registry
                .workers
                .get(&worker_id)
                .is_some_and(|worker| !worker.closed)
            {
                registry
                    .outbound
                    .entry(worker_id)
                    .or_default()
                    .push(encoded);
            }
            Ok(())
        })();
        if let Err(error) = result {
            throw_type_error(scope, &error);
        }
    }

    fn dedicated_worker_close_callback(
        scope: &mut v8::PinScope,
        _args: v8::FunctionCallbackArguments,
        _return_value: v8::ReturnValue<v8::Value>,
    ) {
        let global = scope.get_current_context().global(scope);
        let Some(name) = v8::String::new(scope, "__oryn_worker_id") else {
            return;
        };
        let worker_id = global
            .get(scope, name.into())
            .and_then(|value| value.integer_value(scope))
            .filter(|value| *value > 0)
            .map(|value| value as u64);
        if let Some(worker_id) = worker_id
            && let Some(registry) = scope.get_slot_mut::<DedicatedWorkers>()
            && let Some(worker) = registry.workers.get_mut(&worker_id)
        {
            worker.closed = true;
        }
    }

    fn crypto_callback(
        scope: &mut v8::PinScope,
        args: v8::FunctionCallbackArguments,
        mut return_value: v8::ReturnValue<v8::Value>,
    ) {
        let result = (|| -> Result<String, String> {
            let operation = args
                .get(0)
                .to_string(scope)
                .map(|value| value.to_rust_string_lossy(scope))
                .ok_or_else(|| "crypto operation is not string-convertible".to_string())?;
            let input = args
                .get(1)
                .to_string(scope)
                .map(|value| value.to_rust_string_lossy(scope))
                .unwrap_or_default();
            let bytes = match operation.as_str() {
                "random" => {
                    let length = input
                        .parse::<usize>()
                        .map_err(|error| format!("invalid random length: {error}"))?;
                    if length > 65_536 {
                        return Err("getRandomValues is limited to 65536 bytes".into());
                    }
                    let mut bytes = vec![0_u8; length];
                    getrandom::fill(&mut bytes).map_err(|error| error.to_string())?;
                    bytes
                }
                "sha256" => {
                    use sha2::Digest as _;
                    let bytes = BASE64.decode(input).map_err(|error| error.to_string())?;
                    sha2::Sha256::digest(bytes).to_vec()
                }
                _ => return Err(format!("unsupported crypto operation {operation:?}")),
            };
            Ok(BASE64.encode(bytes))
        })();
        match result {
            Ok(result) => {
                if let Some(value) = v8::String::new(scope, &result) {
                    return_value.set(value.into());
                }
            }
            Err(error) => {
                if let Some(error) = v8::String::new(scope, &error) {
                    scope.throw_exception(error.into());
                }
            }
        }
    }

    #[cfg(unix)]
    fn context_callback(
        scope: &mut v8::PinScope,
        args: v8::FunctionCallbackArguments,
        mut return_value: v8::ReturnValue<v8::Value>,
    ) {
        let result = (|| -> Result<String, String> {
            let state = scope
                .get_slot::<ContextState>()
                .cloned()
                .ok_or_else(|| "context service unavailable".to_string())?;
            let request = args
                .get(0)
                .to_string(scope)
                .map(|value| value.to_rust_string_lossy(scope))
                .ok_or_else(|| "context request is not string-convertible".to_string())?;
            let request: ContextServiceRequest =
                serde_json::from_str(&request).map_err(|error| error.to_string())?;
            let response = state
                .client
                .request(request)
                .map_err(|error| error.to_string())?;
            serde_json::to_string(&response).map_err(|error| error.to_string())
        })();
        match result {
            Ok(result) => {
                if let Some(value) = v8::String::new(scope, &result) {
                    return_value.set(value.into());
                }
            }
            Err(error) => {
                if let Some(error) = v8::String::new(scope, &error) {
                    scope.throw_exception(error.into());
                }
            }
        }
    }

    #[cfg(unix)]
    type StartedStream = (u64, u16, Vec<(String, String)>);

    #[cfg(unix)]
    fn execute_platform_request(
        state: &ContextState,
        operation_id: HostOperationId,
        request: PlatformHostRequest,
    ) -> Result<PlatformHostResult, String> {
        use crate::context_service::WebSocketFrame;
        match request {
            PlatformHostRequest::HttpStart { request } => {
                match state
                    .client
                    .request(ContextServiceRequest::HttpStreamStart {
                        page_id: state.page_id,
                        request: HttpStreamRequest {
                            request_id: operation_id.0,
                            method: request.method,
                            url: request.url.clone(),
                            headers: request.headers,
                            body_base64: BASE64.encode(request.body),
                            credentials: request.credentials != "omit",
                        },
                    })
                    .map_err(|error| error.to_string())?
                {
                    ContextServiceResponse::HttpStreamStarted {
                        stream_id,
                        status,
                        headers,
                        ..
                    } => Ok(PlatformHostResult::HttpStarted {
                        stream_id,
                        status,
                        url: request.url,
                        headers,
                    }),
                    _ => Err("context service returned an invalid HTTP start response".into()),
                }
            }
            PlatformHostRequest::HttpRead {
                stream_id,
                max_bytes,
            } => match state
                .client
                .request(ContextServiceRequest::HttpStreamRead {
                    page_id: state.page_id,
                    stream_id,
                    max_bytes,
                })
                .map_err(|error| error.to_string())?
            {
                ContextServiceResponse::HttpStreamChunk {
                    stream_id,
                    body_base64,
                    done,
                } => Ok(PlatformHostResult::HttpChunk {
                    stream_id,
                    bytes: BASE64
                        .decode(body_base64)
                        .map_err(|error| error.to_string())?,
                    done,
                }),
                _ => Err("context service returned an invalid HTTP chunk response".into()),
            },
            PlatformHostRequest::HttpAbort { stream_id } => match state
                .client
                .request(ContextServiceRequest::HttpStreamAbort {
                    page_id: state.page_id,
                    stream_id,
                })
                .map_err(|error| error.to_string())?
            {
                ContextServiceResponse::HttpStreamAborted => Ok(PlatformHostResult::HttpAborted),
                _ => Err("context service returned an invalid HTTP abort response".into()),
            },
            PlatformHostRequest::WebSocketOpen { url, protocols } => match state
                .client
                .request(ContextServiceRequest::WebSocketOpen {
                    page_id: state.page_id,
                    url,
                    protocols,
                })
                .map_err(|error| error.to_string())?
            {
                ContextServiceResponse::WebSocketOpened {
                    socket_id,
                    protocol,
                } => Ok(PlatformHostResult::WebSocketOpened {
                    socket_id,
                    protocol,
                }),
                _ => Err("context service returned an invalid WebSocket open response".into()),
            },
            PlatformHostRequest::WebSocketSend { socket_id, frame } => {
                let frame = match frame {
                    PlatformWebSocketFrame::Text { value } => WebSocketFrame::Text { value },
                    PlatformWebSocketFrame::Binary { bytes } => WebSocketFrame::Binary {
                        base64: BASE64.encode(bytes),
                    },
                    PlatformWebSocketFrame::Close { code, reason } => {
                        WebSocketFrame::Close { code, reason }
                    }
                };
                match state
                    .client
                    .request(ContextServiceRequest::WebSocketSend {
                        page_id: state.page_id,
                        socket_id,
                        frame,
                    })
                    .map_err(|error| error.to_string())?
                {
                    ContextServiceResponse::WebSocketSent => Ok(PlatformHostResult::WebSocketSent),
                    _ => Err("context service returned an invalid WebSocket send response".into()),
                }
            }
            PlatformHostRequest::WebSocketPoll {
                socket_id,
                max_frames,
            } => match state
                .client
                .request(ContextServiceRequest::WebSocketPoll {
                    page_id: state.page_id,
                    socket_id,
                    max_frames,
                })
                .map_err(|error| error.to_string())?
            {
                ContextServiceResponse::WebSocketEvents { frames } => {
                    Ok(PlatformHostResult::WebSocketEvents {
                        frames: frames
                            .into_iter()
                            .map(|frame| match frame {
                                WebSocketFrame::Text { value } => {
                                    PlatformWebSocketFrame::Text { value }
                                }
                                WebSocketFrame::Binary { base64 } => {
                                    PlatformWebSocketFrame::Binary {
                                        bytes: BASE64.decode(base64).unwrap_or_default(),
                                    }
                                }
                                WebSocketFrame::Close { code, reason } => {
                                    PlatformWebSocketFrame::Close { code, reason }
                                }
                            })
                            .collect(),
                    })
                }
                _ => Err("context service returned an invalid WebSocket poll response".into()),
            },
            PlatformHostRequest::WebSocketClose {
                socket_id,
                code,
                reason,
            } => match state
                .client
                .request(ContextServiceRequest::WebSocketClose {
                    page_id: state.page_id,
                    socket_id,
                    code,
                    reason,
                })
                .map_err(|error| error.to_string())?
            {
                ContextServiceResponse::WebSocketClosed => Ok(PlatformHostResult::WebSocketClosed),
                _ => Err("context service returned an invalid WebSocket close response".into()),
            },
            PlatformHostRequest::IndexedDb { origin, command } => {
                let command: crate::storage::IdbCommand = serde_json::from_value(
                    serde_json::to_value(command).map_err(|error| error.to_string())?,
                )
                .map_err(|error| error.to_string())?;
                match state
                    .client
                    .request(ContextServiceRequest::Storage {
                        request: crate::storage::StorageRequest::IndexedDb { origin, command },
                    })
                    .map_err(|error| error.to_string())?
                {
                    ContextServiceResponse::Storage { response } => match *response {
                        crate::storage::StorageResponse::IndexedDb { response } => {
                            let response: IdbHostResponse = serde_json::from_value(
                                serde_json::to_value(response)
                                    .map_err(|error| error.to_string())?,
                            )
                            .map_err(|error| error.to_string())?;
                            Ok(PlatformHostResult::IndexedDb { response })
                        }
                        _ => {
                            Err("context service returned a non-IndexedDB storage response".into())
                        }
                    },
                    _ => Err("context service returned an invalid IndexedDB response".into()),
                }
            }
        }
    }

    fn start_streamed_request(
        state: &ContextState,
        request: NetworkRequest,
    ) -> Result<StartedStream, String> {
        let started = state
            .client
            .request(ContextServiceRequest::HttpStreamStart {
                page_id: state.page_id,
                request: HttpStreamRequest {
                    request_id: request.request_id,
                    method: request.method,
                    url: request.url,
                    headers: request.headers,
                    body_base64: BASE64.encode(request.body),
                    credentials: request.credentials,
                },
            })
            .map_err(|error| error.to_string())?;
        let ContextServiceResponse::HttpStreamStarted {
            stream_id,
            request_id,
            status,
            headers,
            ..
        } = started
        else {
            return Err("context service returned an invalid HTTP start response".into());
        };
        let _ = request_id;
        Ok((stream_id, status, headers))
    }

    #[cfg(unix)]
    fn stream_read_async_callback(
        scope: &mut v8::PinScope,
        args: v8::FunctionCallbackArguments,
        mut return_value: v8::ReturnValue<v8::Value>,
    ) {
        let started = (|| -> Result<v8::Local<v8::Promise>, String> {
            let context_state = scope
                .get_slot::<ContextState>()
                .cloned()
                .ok_or_else(|| "context service unavailable".to_string())?;
            let stream_id = args
                .get(0)
                .integer_value(scope)
                .ok_or_else(|| "stream identifier is invalid".to_string())?
                as u64;
            let resolver = v8::PromiseResolver::new(scope)
                .ok_or_else(|| "failed to allocate stream promise".to_string())?;
            let promise = resolver.get_promise(scope);
            let context = v8::Global::new(scope, scope.get_current_context());
            let resolver = v8::Global::new(scope, resolver);
            let operations = scope
                .get_slot_mut::<AsyncFetches>()
                .ok_or_else(|| "asynchronous operation registry unavailable".to_string())?;
            let operation_id = operations.next_operation_id;
            operations.next_operation_id = operation_id.saturating_add(1);
            if operations.pending.len() >= 32 {
                return Err("pending host operation limit exceeded".into());
            }
            operations
                .pending
                .insert(operation_id, PendingFetch { context, resolver });
            let sender = operations.sender.clone();
            std::thread::spawn(move || {
                let result = context_state
                    .client
                    .request(ContextServiceRequest::HttpStreamRead {
                        page_id: context_state.page_id,
                        stream_id,
                        max_bytes: 64 * 1024,
                    })
                    .map_err(|error| error.to_string())
                    .and_then(|response| match response {
                        ContextServiceResponse::HttpStreamChunk {
                            stream_id: returned,
                            body_base64,
                            done,
                        } if returned == stream_id => serde_json::to_string(&serde_json::json!({
                            "stream_id": stream_id,
                            "body_base64": body_base64,
                            "done": done,
                        }))
                        .map_err(|error| error.to_string()),
                        _ => Err("context service returned an invalid HTTP chunk".into()),
                    });
                let _ = sender.send(FetchCompletion {
                    operation_id,
                    result,
                });
            });
            Ok(promise)
        })();
        match started {
            Ok(promise) => return_value.set(promise.into()),
            Err(error) => {
                if let Some(error) = v8::String::new(scope, &error) {
                    let exception = v8::Exception::type_error(scope, error);
                    scope.throw_exception(exception);
                }
            }
        }
    }

    fn fetch_async_callback(
        scope: &mut v8::PinScope,
        args: v8::FunctionCallbackArguments,
        mut return_value: v8::ReturnValue<v8::Value>,
    ) {
        #[cfg(unix)]
        let context_state = scope.get_slot::<ContextState>().cloned();
        let started = (|| -> Result<v8::Local<v8::Promise>, String> {
            let fetch_state = scope
                .get_slot::<FetchState>()
                .cloned()
                .ok_or_else(|| "network broker unavailable".to_string())?;
            let input = args
                .get(0)
                .to_string(scope)
                .map(|value| value.to_rust_string_lossy(scope))
                .ok_or_else(|| "fetch URL is not string-convertible".to_string())?;
            let method = args
                .get(1)
                .to_string(scope)
                .map(|value| value.to_rust_string_lossy(scope))
                .unwrap_or_else(|| "GET".into());
            let body_value = args
                .get(2)
                .to_string(scope)
                .map(|value| value.to_rust_string_lossy(scope))
                .unwrap_or_default();
            let body_encoding = args
                .get(4)
                .to_string(scope)
                .map(|value| value.to_rust_string_lossy(scope))
                .unwrap_or_default();
            let body = if body_encoding == "base64" {
                BASE64
                    .decode(body_value)
                    .map_err(|error| format!("invalid base64 request body: {error}"))?
            } else {
                body_value.into_bytes()
            };
            let header_json = args
                .get(3)
                .to_string(scope)
                .map(|value| value.to_rust_string_lossy(scope))
                .unwrap_or_else(|| "[]".into());
            let headers =
                serde_json::from_str::<Vec<(String, String)>>(&header_json).unwrap_or_default();
            let credentials_mode = args
                .get(5)
                .to_string(scope)
                .map(|value| value.to_rust_string_lossy(scope))
                .unwrap_or_else(|| "same-origin".into());
            let resolver = v8::PromiseResolver::new(scope)
                .ok_or_else(|| "failed to allocate fetch promise".to_string())?;
            let promise = resolver.get_promise(scope);
            let context = v8::Global::new(scope, scope.get_current_context());
            let resolver = v8::Global::new(scope, resolver);
            let async_fetches = scope
                .get_slot_mut::<AsyncFetches>()
                .ok_or_else(|| "asynchronous fetch registry unavailable".to_string())?;
            let operation_id = async_fetches.next_operation_id;
            async_fetches.next_operation_id = operation_id.saturating_add(1);
            if async_fetches.pending.len() >= 32 {
                return Err("pending fetch limit exceeded".into());
            }
            async_fetches
                .pending
                .insert(operation_id, PendingFetch { context, resolver });
            let sender = async_fetches.sender.clone();
            std::thread::spawn(move || {
                let result = perform_fetch(
                    fetch_state,
                    #[cfg(unix)]
                    context_state,
                    operation_id,
                    FetchRequest {
                        input,
                        method,
                        body,
                        headers,
                        credentials_mode,
                    },
                );
                let _ = sender.send(FetchCompletion {
                    operation_id,
                    result,
                });
            });
            Ok(promise)
        })();
        match started {
            Ok(promise) => return_value.set(promise.into()),
            Err(error) => {
                if let Some(error) = v8::String::new(scope, &error) {
                    let exception = v8::Exception::type_error(scope, error);
                    scope.throw_exception(exception);
                }
            }
        }
    }

    fn perform_fetch(
        state: FetchState,
        #[cfg(unix)] context_state: Option<ContextState>,
        operation_id: u64,
        request: FetchRequest,
    ) -> Result<String, String> {
        let mut url = Url::parse(&request.input)
            .or_else(|_| state.base_url.join(&request.input))
            .map_err(|error| format!("invalid fetch URL: {error}"))?;
        let request_origin = origin(&state.base_url);
        let mut request_headers = request.headers;
        for redirect in 0..=10_u64 {
            request_headers.retain(|(name, _)| {
                !name.eq_ignore_ascii_case("origin")
                    && !name.eq_ignore_ascii_case("referer")
                    && !name.eq_ignore_ascii_case("cookie")
                    && !name.eq_ignore_ascii_case("authorization")
            });
            request_headers.push(("origin".into(), request_origin.clone()));
            let network_request = NetworkRequest {
                request_id: operation_id.saturating_mul(32).saturating_add(redirect),
                method: request.method.clone(),
                url: url.to_string(),
                headers: request_headers.clone(),
                body: request.body.clone(),
                credentials: request.credentials_mode == "include"
                    || (request.credentials_mode != "omit" && origin(&url) == request_origin),
                security: crate::cookies::CookieRequestContext {
                    credentials: match request.credentials_mode.as_str() {
                        "omit" => crate::cookies::CookieCredentials::Omit,
                        "include" => crate::cookies::CookieCredentials::Include,
                        _ => crate::cookies::CookieCredentials::SameOrigin,
                    },
                    kind: crate::cookies::CookieRequestKind::Fetch,
                    initiator_url: Some(state.base_url.to_string()),
                    top_level_url: Some(state.base_url.to_string()),
                    redirect_chain_cross_site: redirect > 0 && origin(&url) != request_origin,
                    ..Default::default()
                },
                resolved_addrs: Vec::new(),
            };
            #[cfg(unix)]
            let (status, headers, body, response_chunks, stream_id) =
                if let Some(context) = &context_state {
                    let (stream_id, status, headers) =
                        start_streamed_request(context, network_request)?;
                    (status, headers, Vec::new(), Vec::new(), Some(stream_id))
                } else {
                    let response = collect_operation_bounded(
                        &state.broker,
                        network_request,
                        Duration::from_secs(30),
                        16 * 1024 * 1024,
                    )
                    .map_err(|error| error.to_string())?;
                    let chunks = vec![response.body.clone()];
                    (
                        response.status,
                        response.headers,
                        response.body,
                        chunks,
                        None,
                    )
                };
            #[cfg(not(unix))]
            let (status, headers, body, response_chunks, stream_id) = {
                let response = collect_operation_bounded(
                    &state.broker,
                    network_request,
                    Duration::from_secs(30),
                    16 * 1024 * 1024,
                )
                .map_err(|error| error.to_string())?;
                let chunks = vec![response.body.clone()];
                (
                    response.status,
                    response.headers,
                    response.body,
                    chunks,
                    None,
                )
            };
            if (300..400).contains(&status) {
                let location = headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("location"))
                    .map(|(_, value)| value)
                    .ok_or_else(|| "fetch redirect omitted Location".to_string())?;
                url = url
                    .join(location)
                    .map_err(|error| format!("invalid fetch redirect: {error}"))?;
                #[cfg(unix)]
                if let (Some(context), Some(stream_id)) = (&context_state, stream_id) {
                    context
                        .client
                        .request(ContextServiceRequest::HttpStreamAbort {
                            page_id: context.page_id,
                            stream_id,
                        })
                        .map_err(|error| error.to_string())?;
                }
                continue;
            }
            if origin(&url) != request_origin {
                let allowed_origin = headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("access-control-allow-origin"))
                    .map(|(_, value)| value.as_str())
                    .unwrap_or_default();
                let includes_credentials = request.credentials_mode == "include";
                let credentials_allowed = !includes_credentials
                    || headers.iter().any(|(name, value)| {
                        name.eq_ignore_ascii_case("access-control-allow-credentials")
                            && value.eq_ignore_ascii_case("true")
                    });
                if !((allowed_origin == request_origin
                    || (!includes_credentials && allowed_origin == "*"))
                    && credentials_allowed)
                {
                    return Err(format!(
                        "cross-origin fetch denied by CORS: initiator={request_origin}, response={}",
                        origin(&url)
                    ));
                }
            }
            return serde_json::to_string(&FetchResult {
                url: url.to_string(),
                status,
                headers,
                stream_id,
                body_base64: BASE64.encode(body),
                body_chunks_base64: response_chunks
                    .into_iter()
                    .map(|chunk| BASE64.encode(chunk))
                    .collect(),
            })
            .map_err(|error| error.to_string());
        }
        Err("fetch redirect limit exceeded".into())
    }

    fn origin(url: &Url) -> String {
        let port = url
            .port()
            .map(|port| format!(":{port}"))
            .unwrap_or_default();
        format!(
            "{}://{}{}",
            url.scheme(),
            url.host_str().unwrap_or_default(),
            port
        )
    }

    fn module_source<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        resource_name: &str,
        source: &str,
    ) -> Result<v8::script_compiler::Source, HostError> {
        let source = v8::String::new(scope, source)
            .ok_or_else(|| HostError::Execution("module source allocation failed".into()))?;
        let name = v8::String::new(scope, resource_name)
            .ok_or_else(|| HostError::Execution("module name allocation failed".into()))?;
        let origin = v8::ScriptOrigin::new(
            scope,
            name.into(),
            0,
            0,
            false,
            -1,
            None,
            false,
            false,
            true,
            None,
        );
        Ok(v8::script_compiler::Source::new(source, Some(&origin)))
    }

    #[allow(clippy::unnecessary_wraps)]
    fn resolve_module<'s>(
        context: v8::Local<'s, v8::Context>,
        specifier: v8::Local<'s, v8::String>,
        _import_attributes: v8::Local<'s, v8::FixedArray>,
        referrer: v8::Local<'s, v8::Module>,
    ) -> Option<v8::Local<'s, v8::Module>> {
        v8::callback_scope!(unsafe scope, context);
        let specifier = specifier.to_rust_string_lossy(scope);
        let mut sources = scope.get_slot::<ModuleSources>()?.0.clone();
        let resource_name = scope
            .get_slot::<ModuleNames>()
            .and_then(|names| names.0.get(&referrer.get_identity_hash().get()))
            .cloned()
            .unwrap_or_default();
        let Some(resolved) = resolve_module_name(&resource_name, &specifier, &sources)
            .or_else(|| fetch_module_source(scope, &resource_name, &specifier))
        else {
            if let Some(failures) = scope.get_slot_mut::<ModuleResolutionFailures>() {
                failures.0.push(format!(
                    "cannot resolve {specifier:?} from {resource_name:?}"
                ));
            }
            return None;
        };
        if let Some(module) = scope
            .get_slot::<ModuleCache>()
            .and_then(|cache| cache.0.get(&resolved))
            .cloned()
        {
            return Some(v8::Local::new(scope, module));
        }
        if let Some(updated) = scope.get_slot::<ModuleSources>() {
            sources = updated.0.clone();
        }
        let source = sources.get(&resolved)?.clone();
        let mut source = module_source(scope, &resolved, &source).ok()?;
        let module = v8::script_compiler::compile_module(scope, &mut source)?;
        if let Some(names) = scope.get_slot_mut::<ModuleNames>() {
            names
                .0
                .insert(module.get_identity_hash().get(), resolved.clone());
        }
        let global = v8::Global::new(scope, module);
        scope
            .get_slot_mut::<ModuleCache>()?
            .0
            .insert(resolved, global);
        Some(module)
    }

    fn fetch_module_source(
        scope: &mut v8::PinScope<'_, '_>,
        resource_name: &str,
        specifier: &str,
    ) -> Option<String> {
        if !(specifier.starts_with('.')
            || specifier.starts_with('/')
            || Url::parse(specifier).is_ok())
        {
            return None;
        }
        let fetch = scope.get_slot::<FetchState>()?.clone();
        let base = Url::parse(resource_name)
            .ok()
            .or_else(|| fetch.base_url.join(resource_name).ok())
            .unwrap_or_else(|| fetch.base_url.clone());
        let url = base.join(specifier).ok()?;
        let response = collect_operation_bounded(
            &fetch.broker,
            NetworkRequest {
                request_id: 900_000,
                method: "GET".into(),
                url: url.to_string(),
                headers: vec![("accept".into(), "text/javascript,*/*;q=0.1".into())],
                body: Vec::new(),
                credentials: true,
                security: crate::cookies::CookieRequestContext {
                    kind: crate::cookies::CookieRequestKind::Script,
                    initiator_url: Some(fetch.base_url.to_string()),
                    top_level_url: Some(fetch.base_url.to_string()),
                    ..Default::default()
                },
                resolved_addrs: Vec::new(),
            },
            Duration::from_secs(30),
            8 * 1024 * 1024,
        )
        .ok()?;
        if !(200..300).contains(&response.status) {
            return None;
        }
        let source = String::from_utf8(response.body).ok()?;
        let key = url.to_string();
        scope
            .get_slot_mut::<ModuleSources>()?
            .0
            .insert(key.clone(), source);
        Some(key)
    }

    impl Drop for RawV8Host {
        fn drop(&mut self) {
            // `OwnedIsolate` requires itself to be current during destruction.
            // A BFCache-retained host is deliberately exited while frozen, so
            // re-enter it for the automatic field drop and restore the prior
            // isolate when `OwnedIsolate` exits.
            if !self.isolate_entered {
                // SAFETY: the page actor drops hosts on their creating thread.
                unsafe { self.isolate.enter() };
                self.isolate_entered = true;
            }
        }
    }

    impl Default for RawV8Host {
        fn default() -> Self {
            Self::new()
        }
    }

    impl JavaScriptHost for RawV8Host {
        fn kind(&self) -> HostKind {
            HostKind::RawV8
        }

        fn execute(&mut self, source: &str) -> Result<(), HostError> {
            let _deadline = ExecutionDeadline::arm(&self.isolate, self.execution_wall_limit);
            v8::scope!(let handle_scope, &mut self.isolate);
            let context = v8::Local::new(
                handle_scope,
                self.contexts
                    .get(&self.active_realm)
                    .expect("active realm context exists"),
            );
            let scope = &mut v8::ContextScope::new(handle_scope, context);
            v8::tc_scope!(let scope, scope);
            let source = v8::String::new(scope, source)
                .ok_or_else(|| HostError::Execution("source allocation failed".into()))?;
            let Some(script) = v8::Script::compile(scope, source, None) else {
                let detail = scope
                    .exception()
                    .filter(|value| !value.is_null_or_undefined())
                    .or_else(|| {
                        scope
                            .stack_trace()
                            .filter(|value| !value.is_null_or_undefined())
                    })
                    .and_then(|value| value.to_string(scope))
                    .map(|value| value.to_rust_string_lossy(scope))
                    .unwrap_or_else(|| "compilation failed".into());
                return Err(HostError::Execution(detail));
            };
            if script.run(scope).is_none() {
                if _deadline.expired() || scope.is_execution_terminating() {
                    scope.cancel_terminate_execution();
                    return Err(HostError::LimitExceeded {
                        resource: "wall time".into(),
                    });
                }
                let detail = scope
                    .exception()
                    .filter(|value| !value.is_null_or_undefined())
                    .or_else(|| {
                        scope
                            .stack_trace()
                            .filter(|value| !value.is_null_or_undefined())
                    })
                    .and_then(|value| value.to_string(scope))
                    .map(|value| value.to_rust_string_lossy(scope))
                    .unwrap_or_else(|| "execution failed".into());
                return Err(HostError::Execution(detail));
            }
            scope.perform_microtask_checkpoint();
            if scope.has_caught() {
                let detail = scope
                    .exception()
                    .filter(|value| !value.is_null_or_undefined())
                    .or_else(|| {
                        scope
                            .stack_trace()
                            .filter(|value| !value.is_null_or_undefined())
                    })
                    .and_then(|value| value.to_string(scope))
                    .map(|value| value.to_rust_string_lossy(scope))
                    .unwrap_or_else(|| "microtask checkpoint failed".into());
                return Err(HostError::Execution(detail));
            }
            if _deadline.expired() || scope.is_execution_terminating() {
                scope.cancel_terminate_execution();
                return Err(HostError::LimitExceeded {
                    resource: "wall time".into(),
                });
            }
            Ok(())
        }

        fn evaluate_string(&mut self, source: &str) -> Result<String, HostError> {
            let _deadline = ExecutionDeadline::arm(&self.isolate, self.execution_wall_limit);
            v8::scope!(let handle_scope, &mut self.isolate);
            let context = v8::Local::new(
                handle_scope,
                self.contexts
                    .get(&self.active_realm)
                    .expect("active realm context exists"),
            );
            let scope = &mut v8::ContextScope::new(handle_scope, context);
            v8::tc_scope!(let scope, scope);
            let source = v8::String::new(scope, source)
                .ok_or_else(|| HostError::Execution("source allocation failed".into()))?;
            let script = v8::Script::compile(scope, source, None)
                .ok_or_else(|| HostError::Execution("compilation failed".into()))?;
            let value = match script.run(scope) {
                Some(value) => value,
                None if _deadline.expired() || scope.is_execution_terminating() => {
                    scope.cancel_terminate_execution();
                    return Err(HostError::LimitExceeded {
                        resource: "wall time".into(),
                    });
                }
                None => {
                    let detail = scope
                        .stack_trace()
                        .or_else(|| scope.exception())
                        .and_then(|value| value.to_string(scope))
                        .map(|value| value.to_rust_string_lossy(scope))
                        .unwrap_or_else(|| "execution failed".into());
                    return Err(HostError::Execution(detail));
                }
            };
            let value = value
                .to_string(scope)
                .ok_or_else(|| HostError::Execution("result is not string-convertible".into()))?;
            Ok(value.to_rust_string_lossy(scope))
        }

        fn terminate(&mut self) {
            self.isolate.terminate_execution();
        }
    }
}

#[cfg(feature = "v8-host")]
pub use raw_v8::{
    RawV8Host, STARTUP_EXTERNAL_REFERENCE_NAMES, install_startup_snapshot_globals,
    startup_external_references,
};

#[cfg(all(test, feature = "v8-host"))]
mod tests {
    use std::{cell::RefCell, collections::BTreeMap, rc::Rc, time::Duration};

    use super::{
        HostError, JavaScriptHost, RawV8Host, STARTUP_EXTERNAL_REFERENCE_NAMES,
        startup_external_references,
    };
    use crate::network::{
        NetworkBroker, NetworkError, NetworkRequest, NetworkResponse, SharedNetworkBroker,
    };
    use url::Url;

    struct SlowFixtureBroker;

    impl NetworkBroker for SlowFixtureBroker {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            std::thread::sleep(Duration::from_millis(10));
            Ok(NetworkResponse {
                request_id: request.request_id,
                status: 201,
                headers: vec![("content-type".into(), "text/plain".into())],
                body: b"created".to_vec(),
            })
        }
    }

    #[test]
    fn fetch_host_callback_returns_a_promise_before_broker_completion() {
        let mut host = RawV8Host::new_with_network(
            SharedNetworkBroker::new(SlowFixtureBroker),
            Url::parse("https://example.test/").unwrap(),
        );
        host.execute(
            "globalThis.fetchState='pending';__oryn_fetch_async('/item','POST','','[]','base64','same-origin').then(value=>fetchState=String(JSON.parse(value).status))",
        )
        .unwrap();
        assert_eq!(host.evaluate_string("fetchState").unwrap(), "pending");
        assert!(host.has_pending_async_fetches());
        assert_eq!(host.drain_async_fetches(true).unwrap(), 1);
        assert_eq!(host.evaluate_string("fetchState").unwrap(), "201");
    }

    #[test]
    fn v8_watchdog_terminates_execution_before_process_replacement() {
        let mut host = RawV8Host::new_with_wall_limit(Duration::from_millis(25));
        let result = host.evaluate_string("for(;;){}");
        assert!(
            matches!(result, Err(HostError::LimitExceeded { .. })),
            "{result:?}"
        );
        assert_eq!(host.evaluate_string("1 + 1").unwrap(), "2");
    }

    #[test]
    fn frame_realms_share_an_isolate_but_not_global_state() {
        let mut host = RawV8Host::new();
        host.execute("globalThis.marker='top'").unwrap();
        let frame = host.create_realm("https://example.test", 4, Some(7));
        host.activate_realm(frame).unwrap();
        assert_eq!(host.evaluate_string("typeof marker").unwrap(), "undefined");
        host.execute("globalThis.marker='frame'").unwrap();
        assert_eq!(host.evaluate_string("marker").unwrap(), "frame");
        assert_eq!(host.realm_descriptor(frame).unwrap().document_generation, 4);
        host.activate_realm(oryn_js::realm::RealmId(0)).unwrap();
        assert_eq!(host.evaluate_string("marker").unwrap(), "top");
        assert_eq!(
            host.evaluate_string(
                "__oryn_generated_bindings.some(binding=>binding.name==='Document'&&binding.members.includes('createElement'))"
            )
            .unwrap(),
            "true"
        );
        host.remove_realm(frame).unwrap();
        assert!(host.activate_realm(frame).is_err());
    }

    #[test]
    fn generated_bindings_are_installed_as_executable_v8_templates() {
        let parsed = oryn_dom::html::parse("<main id='root'></main>");
        let dom = Rc::new(RefCell::new(parsed.into_runtime().unwrap()));
        let mut host = RawV8Host::new();
        host.install_dom(dom.clone(), 1).unwrap();
        assert_eq!(
            host.evaluate_string(
                "const n=document.createElement('button');n.id='go';n.textContent='Run';document.querySelector('#root').appendChild(n);String(n===document.getElementById('go'))"
            )
            .unwrap(),
            "true"
        );
        let button = dom.borrow().find_by_id("go").unwrap();
        assert_eq!(dom.borrow().arena().text_content(button).unwrap(), "Run");
    }

    #[test]
    fn shared_and_service_worker_realms_report_their_distinct_global_scope() {
        let mut host = RawV8Host::new();
        for (kind, expected) in [
            ("sharedworker", "SharedWorkerGlobalScope"),
            ("serviceworker", "ServiceWorkerGlobalScope"),
        ] {
            let source = serde_json::to_string(
                "postMessage({scope:__oryn_worker_global_name,tag:Object.prototype.toString.call(self),matches:self instanceof globalThis[__oryn_worker_global_name]})",
            )
            .unwrap();
            let kind = serde_json::to_string(kind).unwrap();
            let id = host
                .evaluate_string(&format!(
                    "__oryn_worker_create({source},'https://example.test/worker.js',false,{kind})"
                ))
                .unwrap();
            let messages = host
                .evaluate_string(&format!("__oryn_worker_take({id})"))
                .unwrap();
            let messages: Vec<String> = serde_json::from_str(&messages).unwrap();
            let payload: serde_json::Value = serde_json::from_str(&messages[0]).unwrap();
            assert_eq!(payload["scope"], expected);
            assert_eq!(payload["tag"], format!("[object {expected}]"));
            assert_eq!(payload["matches"], true);
        }
    }

    #[test]
    fn frame_documents_use_generated_bindings_in_contexts_of_one_isolate() {
        let top = Rc::new(RefCell::new(
            oryn_dom::html::parse("<main id='top'></main>")
                .into_runtime()
                .unwrap(),
        ));
        let frame = Rc::new(RefCell::new(
            oryn_dom::html::parse("<main id='frame'></main>")
                .into_runtime()
                .unwrap(),
        ));
        let mut host = RawV8Host::new();
        host.install_dom(top.clone(), 1).unwrap();
        let frame_realm = host
            .create_realm_with_dom("https://example.test", 1, Some(7), frame.clone())
            .unwrap();
        host.activate_realm(frame_realm).unwrap();
        host.execute("document.getElementById('frame').textContent='child'")
            .unwrap();
        host.activate_realm(oryn_js::realm::RealmId(0)).unwrap();
        host.execute("document.getElementById('top').textContent='parent'")
            .unwrap();
        assert_eq!(
            top.borrow()
                .arena()
                .text_content(top.borrow().find_by_id("top").unwrap())
                .unwrap(),
            "parent"
        );
        assert_eq!(
            frame
                .borrow()
                .arena()
                .text_content(frame.borrow().find_by_id("frame").unwrap())
                .unwrap(),
            "child"
        );
    }

    #[test]
    fn webassembly_is_available_in_the_v8_runtime_profile() {
        let mut host = RawV8Host::new();
        assert_eq!(
            host.evaluate_string("typeof WebAssembly + ':' + __oryn_runtime_profile")
                .unwrap(),
            if cfg!(feature = "core-lite") {
                if option_env!("ORYN_ICU_DATA_SHA256").is_some() {
                    "object:core-lite"
                } else {
                    "object:core-lite-unattested-probe"
                }
            } else {
                "object:core-full"
            }
        );
        assert_eq!(
            host.evaluate_string("WebAssembly.validate(new Uint8Array([0,97,115,109,1,0,0,0]))")
                .unwrap(),
            "true"
        );
    }

    #[test]
    fn startup_external_reference_manifest_matches_runtime_order() {
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../assets/external-references-v1.json"))
                .expect("external-reference manifest");
        let names = manifest["references"]
            .as_array()
            .expect("reference array")
            .iter()
            .map(|value| value.as_str().expect("reference name"))
            .collect::<Vec<_>>();
        assert_eq!(names, STARTUP_EXTERNAL_REFERENCE_NAMES);
        assert_eq!(startup_external_references().len(), names.len());
    }

    #[cfg(feature = "core-lite")]
    #[test]
    fn core_lite_never_claims_reduction_without_a_reduced_icu_artifact() {
        let mut host = RawV8Host::new();
        assert_eq!(
            host.evaluate_string("new Intl.NumberFormat('en-US').format(12)")
                .unwrap(),
            "12"
        );
        assert_eq!(
            host.evaluate_string("__oryn_runtime_profile").unwrap(),
            if option_env!("ORYN_ICU_DATA_SHA256").is_some() {
                "core-lite"
            } else {
                "core-lite-unattested-probe"
            }
        );
        if option_env!("ORYN_ICU_DATA_SHA256").is_some() {
            assert_eq!(
                host.evaluate_string(
                    "JSON.stringify(Intl.NumberFormat.supportedLocalesOf(['en','en-US','fr','ja']))"
                )
                .unwrap(),
                "[\"en\",\"en-US\"]"
            );
            assert_eq!(
                host.evaluate_string(
                    "new Intl.Segmenter('en-US',{granularity:'word'}).segment('one two').containing(1).segment"
                )
                .unwrap(),
                "one"
            );
        }
    }

    #[cfg(oryn_startup_snapshot)]
    #[test]
    fn startup_snapshot_is_loaded_with_matching_external_references() {
        let mut host = RawV8Host::new();
        assert_eq!(
            host.evaluate_string(
                "__oryn_snapshot_version+':'+__oryn_snapshot_binding_abi+':'+typeof __oryn_crypto_sync"
            )
            .unwrap(),
            "2:1:function"
        );
        assert!(option_env!("ORYN_STARTUP_SNAPSHOT_SHA256").is_some());
        assert!(option_env!("ORYN_EXTERNAL_REFERENCES_SHA256").is_some());
    }

    #[test]
    fn static_modules_and_import_map_aliases_share_one_module_instance() {
        let mut host = RawV8Host::new();
        let mut sources = BTreeMap::new();
        sources.insert(
            "https://app.example.test/shared.js".into(),
            "globalThis.moduleInitializations=(globalThis.moduleInitializations||0)+1;export const state={count:0};".into(),
        );
        sources.insert(
            "oryn:module-alias:#shared".into(),
            "https://app.example.test/shared.js".into(),
        );
        host.execute_module(
            "https://app.example.test/first.js",
            "import {state} from '#shared';state.count++;globalThis.first=state;",
            &sources,
        )
        .expect("execute first module");
        host.execute_module(
            "https://app.example.test/second.js",
            "import {state} from './shared.js';state.count++;globalThis.second=state;",
            &sources,
        )
        .expect("execute second module");

        assert_eq!(
            host.evaluate_string("moduleInitializations+'|'+(first===second)+'|'+second.count")
                .expect("read singleton state"),
            "1|true|2"
        );
    }
}
