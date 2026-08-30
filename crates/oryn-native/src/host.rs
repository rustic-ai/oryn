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
        cell::RefCell,
        collections::BTreeMap,
        rc::Rc,
        sync::{
            Arc, Once,
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
    use crate::network::{NetworkBroker, NetworkRequest, SharedNetworkBroker};
    #[cfg(unix)]
    use oryn_common::v3::PageId;
    use oryn_dom::NodeId;
    use oryn_js::realm::{RealmDescriptor, RealmId, RealmRegistry};
    use oryn_js::scheduler::{PageScheduler, Task, TaskSource};
    use oryn_js::v8_bindings::{
        BindingLayout, SharedDom, V8BindingState, deliver_mutation_observers,
        install_generated_bindings, install_generated_bindings_with_root, wrap_node,
    };

    static INITIALIZE_V8: Once = Once::new();

    pub struct RawV8Host {
        isolate: v8::OwnedIsolate,
        contexts: BTreeMap<RealmId, v8::Global<v8::Context>>,
        realms: RealmRegistry,
        active_realm: RealmId,
        binding_states: BTreeMap<RealmId, Rc<V8BindingState>>,
        trusted_types_config: Option<(oryn_js::trusted_types::TrustedTypesConfig, bool)>,
        execution_wall_limit: Duration,
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
            "Object.defineProperty(globalThis,'__oryn_generated_bindings',{{value:Object.freeze({encoded}),enumerable:false}});"
        );
        let source = v8::String::new(scope, &source).expect("allocate generated binding metadata");
        let script = v8::Script::compile(scope, source, None)
            .expect("compile generated binding metadata installer");
        script
            .run(scope)
            .expect("install generated binding metadata");
    }

    fn install_runtime_profile(scope: &mut v8::ContextScope<v8::HandleScope>) {
        #[cfg(feature = "core-lite")]
        {
            const SOURCE: &str = r#"
                (() => {
                  const allowed = locale => locale === undefined || locale === null ||
                    ['', 'root', 'en', 'en-US', 'en-us'].includes(String(locale));
                  const validate = locales => {
                    if (locales === undefined) return;
                    const values = Array.isArray(locales) ? locales : [locales];
                    for (const locale of values) {
                      if (!allowed(locale)) throw new RangeError(
                        `Intl locale ${String(locale)} is unavailable in core-lite; supported locales are root, en, and en-US`
                      );
                    }
                  };
                  for (const name of ['Collator','DateTimeFormat','DisplayNames','ListFormat','Locale','NumberFormat','PluralRules','RelativeTimeFormat','Segmenter']) {
                    const Native = Intl[name];
                    if (typeof Native !== 'function') continue;
                    const Wrapped = new Proxy(Native, {
                      apply(target, self, args) { validate(args[0]); return Reflect.apply(target, self, args); },
                      construct(target, args, receiver) { validate(args[0]); return Reflect.construct(target, args, receiver); }
                    });
                    if (typeof Native.supportedLocalesOf === 'function') {
                      Wrapped.supportedLocalesOf = locales => {
                        validate(locales);
                        return Native.supportedLocalesOf(locales);
                      };
                    }
                    Intl[name] = Wrapped;
                  }
                  Object.defineProperty(globalThis, '__oryn_runtime_profile', { value: 'core-lite' });
                })();
            "#;
            let source = v8::String::new(scope, SOURCE).expect("allocate core-lite startup source");
            let script = v8::Script::compile(scope, source, None)
                .expect("compile core-lite Intl capability guard");
            script
                .run(scope)
                .expect("install core-lite Intl capability guard");
        }
        #[cfg(not(feature = "core-lite"))]
        {
            let name = v8::String::new(scope, "__oryn_runtime_profile")
                .expect("allocate runtime profile name");
            let value =
                v8::String::new(scope, "core-full").expect("allocate runtime profile value");
            scope
                .get_current_context()
                .global(scope)
                .create_data_property(scope, name.into(), value.into());
        }
    }

    impl RawV8Host {
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

        pub fn new() -> Self {
            Self::create(None, None, 256 * 1024 * 1024)
        }

        pub fn new_with_network(broker: SharedNetworkBroker, base_url: Url) -> Self {
            Self::create(
                Some(FetchState { broker, base_url }),
                None,
                256 * 1024 * 1024,
            )
        }

        pub fn new_with_network_and_heap_limit(
            broker: SharedNetworkBroker,
            base_url: Url,
            max_heap_bytes: usize,
        ) -> Self {
            Self::create(Some(FetchState { broker, base_url }), None, max_heap_bytes)
        }

        #[cfg(unix)]
        pub fn new_with_network_context_and_heap_limit(
            broker: Option<SharedNetworkBroker>,
            base_url: Option<Url>,
            context: SharedContextClient,
            page_id: PageId,
            max_heap_bytes: usize,
        ) -> Self {
            let fetch = broker
                .zip(base_url)
                .map(|(broker, base_url)| FetchState { broker, base_url });
            Self::create(
                fetch,
                Some(ContextState {
                    client: context,
                    page_id,
                }),
                max_heap_bytes,
            )
        }

        pub fn new_with_heap_limit(max_heap_bytes: usize) -> Self {
            Self::create(None, None, max_heap_bytes)
        }

        #[cfg(test)]
        pub fn new_with_wall_limit(execution_wall_limit: Duration) -> Self {
            let mut host = Self::create(None, None, 256 * 1024 * 1024);
            host.execution_wall_limit = execution_wall_limit;
            host
        }

        fn create(
            fetch_state: Option<FetchState>,
            #[cfg(unix)] context_state: Option<ContextState>,
            #[cfg(not(unix))] _context_state: Option<()>,
            max_heap_bytes: usize,
        ) -> Self {
            INITIALIZE_V8.call_once(|| {
                let platform = v8::new_default_platform(0, false).make_shared();
                v8::V8::initialize_platform(platform);
                v8::V8::initialize();
            });
            let mut isolate =
                v8::Isolate::new(v8::CreateParams::default().heap_limits(0, max_heap_bytes));
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
            let origin = isolate
                .get_slot::<FetchState>()
                .map(|state| state.base_url.origin().ascii_serialization())
                .unwrap_or_else(|| "null".into());
            let mut realms = RealmRegistry::default();
            let active_realm = realms.create(origin, 1, None);
            let context = {
                v8::scope!(let scope, &mut isolate);
                let context = v8::Context::new(scope, Default::default());
                install_host_functions!(scope, context);
                v8::Global::new(scope, context)
            };
            let mut contexts = BTreeMap::new();
            contexts.insert(active_realm, context);
            Self {
                isolate,
                contexts,
                realms,
                active_realm,
                binding_states: BTreeMap::new(),
                trusted_types_config: None,
                execution_wall_limit: Duration::from_millis(9_500),
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
            if let Some((config, allow_eval)) = &self.trusted_types_config {
                state.configure_trusted_types(config.clone());
                context.set_allow_generation_from_strings(*allow_eval);
            }
            self.binding_states.insert(realm, state);
            Ok(())
        }

        pub fn install_dom_root(
            &mut self,
            dom: SharedDom,
            document_generation: u64,
            document: NodeId,
        ) -> Result<(), HostError> {
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

        pub fn set_document_write_anchor(
            &mut self,
            anchor: Option<NodeId>,
        ) -> Result<(), HostError> {
            self.binding_states
                .get(&self.active_realm)
                .ok_or_else(|| HostError::Execution("active Rust DOM is unavailable".into()))?
                .set_document_write_anchor(anchor);
            Ok(())
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

        pub fn install_layout(&mut self, layout: BindingLayout) -> Result<(), HostError> {
            self.binding_states
                .get(&self.active_realm)
                .ok_or_else(|| HostError::Execution("active Rust DOM is unavailable".into()))?
                .set_layout(layout);
            Ok(())
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
                    scope.perform_microtask_checkpoint();
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
            Ok(delivered)
        }

        pub fn has_pending_async_fetches(&self) -> bool {
            self.isolate
                .get_slot::<AsyncFetches>()
                .is_some_and(|fetches| !fetches.pending.is_empty())
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
            let realm = self.realms.create(origin, document_generation, frame_id);
            let context = {
                v8::scope!(let scope, &mut self.isolate);
                let context = v8::Context::new(scope, Default::default());
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
            constructor(type, init = {}) { this.type = String(type); Object.assign(this, init); }
          }
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
          globalThis.__oryn_dispatch_message = encoded => {
            const event = new MessageEvent('message', { data: JSON.parse(encoded) });
            dispatchEvent(event);
          };
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
                let url = serde_json::to_string(&resource_name).map_err(|e| e.to_string())?;
                let bootstrap = format!(
                    "globalThis.location=Object.freeze({{href:{url}}});{}",
                    DEDICATED_WORKER_BOOTSTRAP
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
                    let response = state
                        .broker
                        .send(network_request)
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
                let response = state
                    .broker
                    .lock()
                    .map_err(|_| "network broker lock poisoned".to_string())?
                    .send(network_request)
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
        let response = fetch
            .broker
            .send(NetworkRequest {
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
            })
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
                    .stack_trace()
                    .or_else(|| scope.exception())
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
                    .stack_trace()
                    .or_else(|| scope.exception())
                    .and_then(|value| value.to_string(scope))
                    .map(|value| value.to_rust_string_lossy(scope))
                    .unwrap_or_else(|| "execution failed".into());
                return Err(HostError::Execution(detail));
            }
            scope.perform_microtask_checkpoint();
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
pub use raw_v8::RawV8Host;

#[cfg(all(test, feature = "v8-host"))]
mod tests {
    use std::{cell::RefCell, collections::BTreeMap, rc::Rc, time::Duration};

    use super::{HostError, JavaScriptHost, RawV8Host};
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
                "object:core-lite"
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

    #[cfg(feature = "core-lite")]
    #[test]
    fn core_lite_rejects_unavailable_intl_locales_explicitly() {
        let mut host = RawV8Host::new();
        assert_eq!(
            host.evaluate_string("new Intl.NumberFormat('en-US').format(12)")
                .unwrap(),
            "12"
        );
        let error = host
            .evaluate_string("new Intl.NumberFormat('fr-FR').format(12)")
            .unwrap_err();
        assert!(error.to_string().contains("core-lite"), "{error}");
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

#[cfg(feature = "deno-host")]
mod deno {
    use super::{HostError, HostKind, JavaScriptHost};

    pub struct DenoCoreHost {
        runtime: deno_core::JsRuntime,
    }

    impl DenoCoreHost {
        pub fn new() -> Self {
            Self {
                runtime: deno_core::JsRuntime::new(deno_core::RuntimeOptions::default()),
            }
        }
    }

    impl Default for DenoCoreHost {
        fn default() -> Self {
            Self::new()
        }
    }

    impl JavaScriptHost for DenoCoreHost {
        fn kind(&self) -> HostKind {
            HostKind::DenoCore
        }

        fn execute(&mut self, source: &str) -> Result<(), HostError> {
            self.runtime
                .execute_script("oryn:probe", source.to_string())
                .map(|_| ())
                .map_err(|error| HostError::Execution(error.to_string()))
        }

        fn evaluate_string(&mut self, _source: &str) -> Result<String, HostError> {
            Err(HostError::Execution(
                "string evaluation is not implemented by the comparison host".into(),
            ))
        }

        fn terminate(&mut self) {
            self.runtime.v8_isolate().terminate_execution();
        }
    }
}

#[cfg(feature = "deno-host")]
pub use deno::DenoCoreHost;
