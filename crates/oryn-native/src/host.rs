/// Decoded navigation chunks and the response type that selects their parser.
#[cfg_attr(not(feature = "v8-host"), allow(dead_code))]
pub(crate) struct DocumentStream {
    pub chunks: Box<dyn Iterator<Item = Result<String, HostError>>>,
    pub content_type: Option<String>,
    pub encoding: std::sync::Arc<std::sync::Mutex<Option<&'static str>>>,
}

#[cfg_attr(not(feature = "v8-host"), allow(dead_code))]
impl DocumentStream {
    pub fn html(chunks: Box<dyn Iterator<Item = Result<String, HostError>>>) -> Self {
        Self {
            chunks,
            content_type: None,
            encoding: Default::default(),
        }
    }
}

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
            Arc, Condvar, Mutex, Once,
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
    use oryn_js::origin::Origin;
    #[cfg(unix)]
    use oryn_js::platform::{
        HostOperationId, IdbHostResponse, PlatformHost, PlatformHostCompletion,
        PlatformHostRequest, PlatformHostResult, PlatformWebSocketFrame,
    };
    use oryn_js::realm::{
        AgentEnvironments, EnvironmentDescriptor, EnvironmentId, RealmDescriptor, RealmId,
        RealmRegistry,
    };
    use oryn_js::scheduler::{PageScheduler, Task, TaskSource};
    use oryn_js::v8_bindings::{
        BindingGlobal, BindingLayout, BindingLayoutProvider, PendingDefaultAction, SharedDom,
        V8BindingState, deliver_control_event, deliver_mutation_observers,
        deliver_selection_change, fixup_focus_eligibility, has_pending_mutation_observer_microtask,
        install_generated_bindings_with_root, install_generated_bindings_with_root_for_global,
        publish_named_window_properties, queue_mutation_observer_microtask,
        reset_mutation_observer_microtask_after_termination, wrap_node,
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
        "import_scripts_callback",
        "automation_trace_callback",
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
            v8::ExternalReference {
                function: import_scripts_callback.map_fn_to(),
            },
            v8::ExternalReference {
                function: automation_trace_callback.map_fn_to(),
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
        watchdog: ExecutionWatchdog,
        isolate: v8::OwnedIsolate,
        contexts: BTreeMap<RealmId, v8::Global<v8::Context>>,
        active_realm: RealmId,
        binding_states: BTreeMap<RealmId, Rc<V8BindingState>>,
        execution_wall_limit: Duration,
        isolate_entered: bool,
    }

    /// Immutable policy owned by a native document context, never by the
    /// isolate's most recently configured document.
    #[derive(Clone)]
    struct DocumentCodePolicy {
        trusted_types: oryn_js::trusted_types::TrustedTypesConfig,
        allow_eval: bool,
    }

    /// Identity allocation belongs to the isolate so native callbacks can create
    /// realms without borrowing (or retaining a pointer to) the host driver.
    #[derive(Default)]
    struct NativeRealmIdentities {
        realms: RealmRegistry,
        security_tokens: BTreeMap<String, v8::Global<v8::String>>,
        // Strongly retain callback-created contexts until the driver reaches a
        // checkpoint. Identity metadata alone must not outlive an unowned realm.
        pending_contexts: BTreeMap<RealmId, v8::Global<v8::Context>>,
    }

    #[derive(Default)]
    struct DeadlineSchedule {
        next_id: u64,
        shutdown: bool,
        active: BTreeMap<u64, (Instant, Arc<AtomicBool>)>,
    }

    struct WatchdogState {
        schedule: Mutex<DeadlineSchedule>,
        changed: Condvar,
        handle: v8::IsolateHandle,
    }

    /// One deadline thread belongs to the isolate, not to each execution.
    /// The schedule lock serializes expiry with disarming, preventing a stale
    /// watchdog from terminating the next unrelated script.
    struct ExecutionWatchdog {
        state: Arc<WatchdogState>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl ExecutionWatchdog {
        fn new(isolate: &mut v8::OwnedIsolate) -> Self {
            let state = Arc::new(WatchdogState {
                schedule: Mutex::new(DeadlineSchedule::default()),
                changed: Condvar::new(),
                handle: isolate.thread_safe_handle(),
            });
            isolate.set_slot(state.clone());
            let worker = state.clone();
            let thread = std::thread::spawn(move || {
                let mut schedule = worker.schedule.lock().expect("deadline schedule lock");
                loop {
                    if schedule.shutdown {
                        break;
                    }
                    let next = schedule
                        .active
                        .values()
                        .filter(|(_, expired)| !expired.load(Ordering::Acquire))
                        .map(|(deadline, _)| *deadline)
                        .min();
                    let Some(next) = next else {
                        schedule = worker
                            .changed
                            .wait(schedule)
                            .expect("deadline schedule wait");
                        continue;
                    };
                    let now = Instant::now();
                    if next <= now {
                        for (deadline, expired) in schedule.active.values() {
                            if *deadline <= now {
                                expired.store(true, Ordering::Release);
                            }
                        }
                        // Keep the lock until termination is issued: disarm and
                        // the next arm cannot overtake this expired generation.
                        worker.handle.terminate_execution();
                    } else {
                        schedule = worker
                            .changed
                            .wait_timeout(schedule, next - now)
                            .expect("deadline schedule timeout")
                            .0;
                    }
                }
            });
            Self {
                state,
                thread: Some(thread),
            }
        }

        fn shutdown(&mut self) {
            {
                let mut schedule = self.state.schedule.lock().expect("deadline shutdown lock");
                schedule.shutdown = true;
                self.state.changed.notify_all();
            }
            if let Some(thread) = self.thread.take() {
                thread.join().expect("deadline thread panicked");
            }
        }
    }

    impl Drop for ExecutionWatchdog {
        fn drop(&mut self) {
            self.shutdown();
        }
    }

    struct ExecutionDeadline {
        state: Arc<WatchdogState>,
        id: Option<u64>,
        deadline: Option<Instant>,
        expired: Arc<AtomicBool>,
    }

    impl ExecutionDeadline {
        fn arm(isolate: &v8::OwnedIsolate, limit: Duration) -> Self {
            let state = isolate
                .get_slot::<Arc<WatchdogState>>()
                .expect("isolate deadline controller installed")
                .clone();
            let expired = Arc::new(AtomicBool::new(false));
            let mut schedule = state.schedule.lock().expect("deadline arm lock");
            let deadline = Instant::now().checked_add(limit);
            let id = schedule.next_id.checked_add(1).filter(|_| {
                !schedule.shutdown
                    && schedule.active.len() < 64
                    && deadline.is_some()
                    && !schedule
                        .active
                        .values()
                        .any(|(_, expired)| expired.load(Ordering::Acquire))
            });
            if let Some(id) = id {
                schedule.next_id = id;
                schedule
                    .active
                    .insert(id, (deadline.unwrap(), expired.clone()));
                state.changed.notify_all();
            } else {
                // Exhaustion is fail-closed, never identity reuse or an
                // execution without a watchdog.
                expired.store(true, Ordering::Release);
                state.handle.terminate_execution();
            }
            drop(schedule);
            Self {
                state,
                id,
                deadline,
                expired,
            }
        }

        fn expired(&self) -> bool {
            self.expired.load(Ordering::Acquire)
                || self
                    .deadline
                    .is_some_and(|deadline| Instant::now() >= deadline)
        }
    }

    impl Drop for ExecutionDeadline {
        fn drop(&mut self) {
            let mut schedule = self.state.schedule.lock().expect("deadline disarm lock");
            if let Some(id) = self.id {
                schedule.active.remove(&id);
            }
            if self.expired.load(Ordering::Acquire)
                && !schedule
                    .active
                    .values()
                    .any(|(_, expired)| expired.load(Ordering::Acquire))
            {
                // Expiry can race the final result check. Do not let its
                // termination bit poison a later, separately admitted script.
                self.state.handle.cancel_terminate_execution();
            }
            self.state.changed.notify_all();
        }
    }

    #[derive(Clone)]
    struct ModuleSources(BTreeMap<String, String>);

    #[derive(Default)]
    struct ModuleNames(BTreeMap<i32, String>);

    #[derive(Default)]
    struct ModuleCache(BTreeMap<(i32, String), v8::Global<v8::Module>>);

    fn current_realm_identity(scope: &mut v8::PinScope<'_, '_>) -> i32 {
        scope
            .get_current_context()
            .global(scope)
            .get_identity_hash()
            .get()
    }

    #[derive(Default)]
    struct ModuleResolutionFailures(Vec<String>);

    #[derive(Default)]
    struct PromiseRejections(BTreeMap<i32, String>);

    #[derive(Default)]
    struct ModuleErrorReports(Vec<String>);

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
        retired: Arc<AtomicBool>,
        context: ContextState,
        sender: mpsc::Sender<PlatformHostCompletion>,
        receiver: Mutex<mpsc::Receiver<PlatformHostCompletion>>,
        idb_sender: mpsc::Sender<(HostOperationId, PlatformHostRequest)>,
        cancellable_http_starts: Arc<Mutex<BTreeSet<HostOperationId>>>,
    }

    #[cfg(unix)]
    impl ContextPlatformHost {
        fn new(context: ContextState) -> Self {
            let retired = Arc::new(AtomicBool::new(false));
            let idb_retired = retired.clone();
            let (sender, receiver) = mpsc::channel();
            let (idb_sender, idb_receiver) = mpsc::channel();
            let idb_context = context.clone();
            let idb_completions = sender.clone();
            std::thread::Builder::new()
                .name("oryn-platform-indexeddb".into())
                .spawn(move || {
                    while let Ok((operation_id, request)) = idb_receiver.recv() {
                        if idb_retired.load(Ordering::Acquire) {
                            break;
                        }
                        let result = execute_platform_request(&idb_context, operation_id, request);
                        if idb_retired.load(Ordering::Acquire) {
                            break;
                        }
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
                retired,
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
            if self.retired.load(Ordering::Acquire) {
                return Err("execution environment is retired".into());
            }
            if matches!(request, PlatformHostRequest::IndexedDb { .. }) {
                return self
                    .idb_sender
                    .send((operation_id, request))
                    .map_err(|_| "IndexedDB platform service is unavailable".to_string());
            }
            let context = self.context.clone();
            let sender = self.sender.clone();
            let retired = self.retired.clone();
            let is_http_start = matches!(
                request,
                PlatformHostRequest::HttpStart { .. } | PlatformHostRequest::ImageStart { .. }
            );
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
                    if retired.load(Ordering::Acquire) {
                        return;
                    }
                    let result = execute_platform_request(&context, operation_id, request);
                    if is_http_start && let Ok(mut operations) = cancellable_http_starts.lock() {
                        operations.remove(&operation_id);
                    }
                    if retired.load(Ordering::Acquire) {
                        return;
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
            if self.retired.load(Ordering::Acquire) {
                return Vec::new();
            }
            let Ok(receiver) = self.receiver.lock() else {
                return Vec::new();
            };
            (0..limit)
                .filter_map(|_| receiver.try_recv().ok())
                .collect()
        }

        fn retire(&self) {
            self.retired.store(true, Ordering::Release);
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
        environment: EnvironmentDescriptor,
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
        retired: BTreeSet<u64>,
    }

    impl Default for AsyncFetches {
        fn default() -> Self {
            let (sender, receiver) = mpsc::channel();
            Self {
                next_operation_id: 1,
                sender,
                receiver,
                pending: BTreeMap::new(),
                retired: BTreeSet::new(),
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
        environment: EnvironmentDescriptor,
        owner_identity: i32,
        context: v8::Global<v8::Context>,
        binding_state: Option<Rc<V8BindingState>>,
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
        environment: EnvironmentDescriptor,
        context: v8::Global<v8::Context>,
        callback: v8::Global<v8::Function>,
        arguments: Vec<v8::Global<v8::Value>>,
        repeating: bool,
        delay_ms: u64,
        nesting_level: u32,
        scheduled_id: u64,
    }

    struct TimerRegistry {
        started: Instant,
        scheduler: PageScheduler<u64>,
        callbacks: BTreeMap<u64, ScheduledTimer>,
        task_nesting: Rc<std::cell::Cell<u32>>,
    }

    impl Default for TimerRegistry {
        fn default() -> Self {
            Self {
                started: Instant::now(),
                scheduler: PageScheduler::default(),
                callbacks: BTreeMap::new(),
                task_nesting: Rc::new(std::cell::Cell::new(0)),
            }
        }
    }

    struct TimerTaskNesting {
        state: Rc<std::cell::Cell<u32>>,
        previous: u32,
    }

    impl TimerTaskNesting {
        fn enter(state: Rc<std::cell::Cell<u32>>, level: u32) -> Self {
            let previous = state.replace(level);
            Self { state, previous }
        }
    }

    impl Drop for TimerTaskNesting {
        fn drop(&mut self) {
            self.state.set(self.previous);
        }
    }

    fn nested_timer_delay(delay_ms: u64, nesting_level: u32) -> Duration {
        Duration::from_millis(if nesting_level > 5 {
            delay_ms.max(4)
        } else {
            delay_ms
        })
    }

    fn current_environment(scope: &mut v8::PinScope) -> Result<EnvironmentDescriptor, String> {
        let environment = scope
            .get_current_context()
            .get_slot::<EnvironmentDescriptor>()
            .map(|value| *value)
            .ok_or_else(|| "execution environment is unavailable".to_string())?;
        if !scope
            .get_slot::<AgentEnvironments>()
            .is_some_and(|registry| registry.is_live(&environment))
        {
            return Err("execution environment is retired".into());
        }
        Ok(environment)
    }

    fn retire_environment(isolate: &mut v8::Isolate, id: EnvironmentId) {
        let retired = isolate
            .get_slot_mut::<AgentEnvironments>()
            .map(|registry| registry.retire(id))
            .unwrap_or_default();
        let ids = retired
            .iter()
            .map(|entry| entry.id)
            .collect::<BTreeSet<_>>();
        if let Some(registry) = isolate.get_slot_mut::<TimerRegistry>() {
            for environment in &retired {
                registry
                    .scheduler
                    .cancel_document(environment.realm, environment.document_generation);
            }
            registry
                .callbacks
                .retain(|_, timer| !ids.contains(&timer.environment.id));
        }
        if let Some(fetches) = isolate.get_slot_mut::<AsyncFetches>() {
            let pending = fetches
                .pending
                .iter()
                .filter(|(_, fetch)| ids.contains(&fetch.environment.id))
                .map(|(id, _)| *id)
                .collect::<Vec<_>>();
            for id in pending {
                fetches.pending.remove(&id);
                fetches.retired.insert(id);
            }
        }
        if let Some(workers) = isolate.get_slot_mut::<DedicatedWorkers>() {
            let retired_workers = workers
                .workers
                .iter()
                .filter(|(_, worker)| ids.contains(&worker.environment.id))
                .map(|(id, _)| *id)
                .collect::<Vec<_>>();
            for worker in retired_workers {
                if let Some(worker) = workers.workers.remove(&worker)
                    && let Some(state) = worker.binding_state
                {
                    state.retire_execution();
                }
                workers.outbound.remove(&worker);
            }
        }
    }

    /// A browsing-context lifetime survives document navigation, but never a
    /// removal/reinsertion of its embedding element. No author property is used
    /// to decide whether execution services remain authorized.
    struct FrameLifetime {
        frame: NodeId,
        connection_generation: u64,
        parent: std::rc::Weak<V8BindingState>,
        parent_environment: EnvironmentDescriptor,
        environment: std::cell::Cell<EnvironmentDescriptor>,
        child: RefCell<std::rc::Weak<V8BindingState>>,
        closed: std::cell::Cell<bool>,
    }

    #[derive(Default)]
    struct FrameLifetimes {
        live: BTreeMap<(EnvironmentId, NodeId), Rc<FrameLifetime>>,
        discarded_realms: BTreeSet<RealmId>,
    }

    /// Register native browsing-context ownership from an entered V8 scope.
    /// This is shared by driver linking and insertion-time realm creation; it
    /// does not call a JavaScript proxy factory or borrow the outer host.
    fn bind_native_frame_lifetime(
        scope: &mut v8::PinScope<'_, '_, ()>,
        parent: v8::Local<v8::Context>,
        child: v8::Local<v8::Context>,
        frame: NodeId,
    ) -> Result<(), HostError> {
        let parent_environment = *parent
            .get_slot::<EnvironmentDescriptor>()
            .ok_or_else(|| HostError::Execution("parent frame environment unavailable".into()))?;
        let environment = *child
            .get_slot::<EnvironmentDescriptor>()
            .ok_or_else(|| HostError::Execution("child frame environment unavailable".into()))?;
        let active = parent
            .get_slot::<V8BindingState>()
            .ok_or_else(|| HostError::Execution("parent frame bindings unavailable".into()))?;
        let child_state = child
            .get_slot::<V8BindingState>()
            .ok_or_else(|| HostError::Execution("child frame bindings unavailable".into()))?;
        let environments = scope
            .get_slot::<AgentEnvironments>()
            .ok_or_else(|| HostError::Execution("agent environments unavailable".into()))?;
        if !environments.is_live(&parent_environment)
            || !environments.is_live(&environment)
            || active.execution_retired()
            || child_state.execution_retired()
        {
            return Err(HostError::Execution(
                "cannot bind a retired frame environment".into(),
            ));
        }
        let dom = active.dom();
        {
            let dom = dom.borrow();
            // The legacy parser loader can link a frame removed by a parent
            // script before its late initialization. Preserve that path: the
            // existing lifecycle checkpoint immediately retires it. Insertion
            // hooks must instead create the initial realm while connected.
            if dom.arena().node_document(frame).ok().flatten() != Some(active.document_root())
                || !dom.is_element(frame, "http://www.w3.org/1999/xhtml", "iframe")
            {
                return Err(HostError::Execution(
                    "frame does not belong to its native owner document".into(),
                ));
            }
        }
        let lifetime = scope
            .get_slot_mut::<FrameLifetimes>()
            .ok_or_else(|| HostError::Execution("frame lifetimes unavailable".into()))?
            .live
            .entry((parent_environment.id, frame))
            .or_insert_with(|| {
                Rc::new(FrameLifetime {
                    frame,
                    connection_generation: dom.borrow_mut().track_frame_context(frame),
                    parent: Rc::downgrade(&active),
                    parent_environment,
                    environment: std::cell::Cell::new(environment),
                    child: RefCell::new(Rc::downgrade(&child_state)),
                    closed: std::cell::Cell::new(false),
                })
            })
            .clone();
        if lifetime.closed.get()
            || dom.borrow().frame_context_generation(frame) != Some(lifetime.connection_generation)
        {
            return Err(HostError::Execution(
                "cannot rebind a discarded frame lifetime".into(),
            ));
        }
        lifetime.environment.set(environment);
        *lifetime.child.borrow_mut() = Rc::downgrade(&child_state);
        child.set_slot(lifetime);
        active.bind_frame_target_name(frame, &child_state);
        Ok(())
    }

    fn checkpoint_frame_lifetimes(scope: &mut v8::PinScope) {
        loop {
            let Some(registry) = scope.get_slot::<FrameLifetimes>() else {
                return;
            };
            let discarded = registry
                .live
                .iter()
                .filter_map(|(key, frame)| {
                    let parent_live =
                        scope
                            .get_slot::<AgentEnvironments>()
                            .is_some_and(|environments| {
                                environments.is_live(&frame.parent_environment)
                            });
                    let connected = frame.parent.upgrade().is_some_and(|parent| {
                        let dom = parent.dom();
                        let dom = dom.borrow();
                        dom.is_connected(frame.frame).unwrap_or(false)
                            && dom.arena().node_document(frame.frame).ok().flatten()
                                == Some(parent.document_root())
                            && dom.frame_context_generation(frame.frame)
                                == Some(frame.connection_generation)
                            && frame.connection_generation != u64::MAX
                    });
                    (!parent_live || !connected).then(|| (*key, frame.clone()))
                })
                .collect::<Vec<_>>();
            if discarded.is_empty() {
                return;
            }
            for (key, frame) in discarded {
                frame.closed.set(true);
                let environment = frame.environment.get();
                retire_environment(scope, environment.id);
                if let Some(child) = frame.child.borrow().upgrade() {
                    child.retire_execution();
                }
                if let Some(parent) = frame.parent.upgrade() {
                    parent.discard_frame_context(frame.frame);
                }
                let registry = scope.get_slot_mut::<FrameLifetimes>().unwrap();
                registry.live.remove(&key);
                registry.discarded_realms.insert(environment.realm);
            }
            // A parent discard also retires nested contexts, including opaque
            // frames with separate arenas whose document roots remain connected.
        }
    }

    fn frame_closed_callback(
        scope: &mut v8::PinScope,
        args: v8::FunctionCallbackArguments,
        mut result: v8::ReturnValue,
    ) {
        let closed = v8::Local::<v8::Object>::try_from(args.data())
            .ok()
            .and_then(|data| data.get_internal_field(scope, 0))
            .and_then(|value| v8::Local::<v8::Context>::try_from(value).ok())
            .and_then(|context| context.get_slot::<FrameLifetime>())
            .is_none_or(|frame| frame.closed.get());
        result.set_bool(closed);
    }

    fn frame_close_callback(
        _scope: &mut v8::PinScope,
        _args: v8::FunctionCallbackArguments,
        _result: v8::ReturnValue,
    ) {
        // A nested browsing context is not script-closable.
    }

    fn install_frame_closed_accessor(
        scope: &mut v8::PinScope,
        object: v8::Local<v8::Object>,
        context: v8::Local<v8::Context>,
    ) -> Result<(), HostError> {
        let template = v8::ObjectTemplate::new(scope);
        template.set_internal_field_count(1);
        let data = template
            .new_instance(scope)
            .ok_or_else(|| HostError::Execution("frame lifecycle data allocation".into()))?;
        data.set_internal_field(0, context.into());
        let getter = v8::Function::builder(frame_closed_callback)
            .data(data.into())
            .build(scope)
            .ok_or_else(|| HostError::Execution("frame closed getter allocation".into()))?;
        let key = v8::String::new(scope, "closed").unwrap();
        let mut descriptor =
            v8::PropertyDescriptor::new_from_get_set(getter.into(), v8::undefined(scope).into());
        descriptor.set_enumerable(true);
        descriptor.set_configurable(true);
        if object.define_property(scope, key.into(), &descriptor) != Some(true) {
            return Err(HostError::Execution(
                "install native frame closed getter".into(),
            ));
        }
        let close = v8::Function::new(scope, frame_close_callback)
            .ok_or_else(|| HostError::Execution("frame close callback allocation".into()))?;
        let key = v8::String::new(scope, "close").unwrap();
        object.set(scope, key.into(), close.into());
        Ok(())
    }

    fn bind_document_environment(
        scope: &mut v8::PinScope,
        realm: RealmId,
        generation: u64,
    ) -> Result<(), HostError> {
        let current = current_environment(scope).map_err(HostError::Execution)?;
        if current.realm == realm && current.document_generation == generation {
            return Ok(());
        }
        let replacement = scope
            .get_slot_mut::<AgentEnvironments>()
            .unwrap()
            .create(realm, generation, current.owner)
            .map_err(|error| HostError::Execution(error.into()))?;
        retire_environment(scope, current.id);
        scope.get_current_context().set_slot(Rc::new(replacement));
        Ok(())
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
            let function = v8::Function::new(context_scope, automation_trace_callback)
                .expect("allocate automation trace callback");
            let name = v8::String::new(context_scope, "__oryn_record_automation_event").unwrap();
            $context.global(context_scope).define_own_property(
                context_scope,
                name.into(),
                function.into(),
                v8::PropertyAttribute::READ_ONLY
                    | v8::PropertyAttribute::DONT_DELETE
                    | v8::PropertyAttribute::DONT_ENUM,
            );
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
            let function = v8::Function::new(context_scope, import_scripts_callback)
                .expect("allocate importScripts callback");
            let name = v8::String::new(context_scope, "__oryn_import_scripts")
                .expect("allocate importScripts callback name");
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
            let environment = match current_environment(scope) {
                Ok(environment) => environment,
                // Detached Window timers must not throw. The retired endpoint
                // returns a numeric no-op handle without retaining a callback.
                Err(error) if error == "execution environment is retired" => return Ok(0),
                Err(error) => return Err(error),
            };
            let callback = v8::Local::<v8::Function>::try_from(args.get(0))
                .map_err(|_| "timer callback must be callable".to_string())?;
            // WebIDL `long` conversion wraps modulo 2^32 before HTML
            // clamps negative timeouts to zero. Preserve conversion exceptions.
            let Some(delay) = args.get(1).int32_value(scope) else {
                return Ok(0);
            };
            let delay = delay.max(0) as u64;
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
            let nesting_level = registry.task_nesting.get();
            // Repeating timers are reinitialized only after their callback and
            // its microtask checkpoint, with the same public handle.
            let timer_id = registry.scheduler.set_timeout_with(
                nested_timer_delay(delay, nesting_level),
                |timer_id| Task {
                    source: TaskSource::Timer,
                    realm: environment.realm,
                    document_generation: environment.document_generation,
                    action_id: None,
                    payload: timer_id,
                },
            );
            registry.callbacks.insert(
                timer_id,
                ScheduledTimer {
                    environment,
                    context,
                    callback,
                    arguments,
                    repeating,
                    delay_ms: delay,
                    nesting_level: nesting_level.saturating_add(1),
                    scheduled_id: timer_id,
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
        let Ok(environment) = current_environment(scope) else {
            return;
        };
        if let Some(registry) = scope.get_slot_mut::<TimerRegistry>() {
            let timer_id = timer_id as u64;
            if !registry
                .callbacks
                .get(&timer_id)
                .is_some_and(|timer| timer.environment.id == environment.id)
            {
                return;
            }
            if let Some(timer) = registry.callbacks.remove(&timer_id) {
                registry.scheduler.cancel_timer(timer.scheduled_id);
            }
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

    /// The shared realm factory works inside a running native callback. No
    /// author code runs here and no borrow of isolate-owned state crosses a V8
    /// allocation. Realm and environment identities remain monotonic on failure.
    fn create_native_realm_context(
        scope: &mut v8::PinScope<'_, '_, ()>,
        origin: Origin,
        document_generation: u64,
        frame_id: Option<u64>,
        global: Option<v8::Global<v8::Object>>,
    ) -> Result<(RealmId, v8::Global<v8::Context>), HostError> {
        let realm = scope
            .get_slot_mut::<NativeRealmIdentities>()
            .unwrap()
            .realms
            .try_create(origin.clone(), document_generation, frame_id)
            .map_err(|_| HostError::LimitExceeded {
                resource: "realm identities".into(),
            })?;
        let environment = match scope.get_slot_mut::<AgentEnvironments>().unwrap().create(
            realm,
            document_generation,
            None,
        ) {
            Ok(environment) => environment,
            Err(error) => {
                scope
                    .get_slot_mut::<NativeRealmIdentities>()
                    .unwrap()
                    .realms
                    .remove(realm);
                return Err(HostError::Execution(error.into()));
            }
        };
        let identity = origin.security_token_key();
        let existing_token = scope
            .get_slot::<NativeRealmIdentities>()
            .unwrap()
            .security_tokens
            .get(&identity)
            .cloned();
        let global_object = global
            .as_ref()
            .map(|value| v8::Local::new(scope, value).into());
        let template = v8::ObjectTemplate::new(scope);
        oryn_js::v8_bindings::window_proxy::configure_local_template(scope, template);
        let context = v8::Context::new(
            scope,
            v8::ContextOptions {
                global_object,
                global_template: Some(template),
                ..Default::default()
            },
        );
        let token = existing_token
            .as_ref()
            .map(|token| v8::Local::new(scope, token))
            .unwrap_or_else(|| {
                v8::String::new(scope, &identity).expect("allocate V8 context security token")
            });
        context.set_security_token(token.into());
        if existing_token.is_none() {
            let token = v8::Global::new(scope, token);
            scope
                .get_slot_mut::<NativeRealmIdentities>()
                .unwrap()
                .security_tokens
                .insert(identity, token);
        }
        let descriptor = scope
            .get_slot::<NativeRealmIdentities>()
            .unwrap()
            .realms
            .get(realm)
            .unwrap()
            .clone();
        context.set_slot(Rc::new(descriptor));
        context.set_slot(Rc::new(environment));
        install_host_functions!(scope, context);
        let context = v8::Global::new(scope, context);
        scope
            .get_slot_mut::<NativeRealmIdentities>()
            .unwrap()
            .pending_contexts
            .insert(realm, context.clone());
        Ok((realm, context))
    }

    /// Install one document through the same path for driver-created and
    /// callback-created realms. Policy is explicit: this helper must not guess
    /// inheritance from whichever realm last happened to run in the isolate.
    fn install_native_document(
        scope: &mut v8::PinScope<'_, '_, ()>,
        context: v8::Local<v8::Context>,
        dom: SharedDom,
        realm: RealmId,
        document_generation: u64,
        document: NodeId,
        policy: Option<&DocumentCodePolicy>,
    ) -> Result<Rc<V8BindingState>, HostError> {
        let scope = &mut v8::ContextScope::new(scope, context);
        let environment = current_environment(scope).map_err(HostError::Execution)?;
        if environment.realm != realm {
            return Err(HostError::Execution(
                "document realm identity mismatch".into(),
            ));
        }
        let previous = context.get_slot::<V8BindingState>();
        #[cfg(unix)]
        let platform_context = scope.get_slot::<ContextState>().cloned();
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
        if let Some(policy) = policy {
            state.configure_trusted_types(policy.trusted_types.clone());
            context.set_allow_generation_from_strings(policy.allow_eval);
            context.set_slot(Rc::new(policy.clone()));
        }
        if let Some(previous) = previous {
            previous.retire_execution();
        }
        bind_document_environment(scope, realm, document_generation)?;
        Ok(state)
    }

    /// Allocate and initialize a child document without activating the outer
    /// driver. The creator's context, not the last configured sibling, supplies
    /// inherited policy. Failed installation must not leave an adoptable realm.
    fn create_native_document_context(
        scope: &mut v8::PinScope<'_, '_, ()>,
        creator: v8::Local<v8::Context>,
        origin: Origin,
        document_generation: u64,
        frame_id: Option<u64>,
        dom: SharedDom,
        document: NodeId,
    ) -> Result<(RealmId, v8::Global<v8::Context>), HostError> {
        let creator_environment = creator
            .get_slot::<EnvironmentDescriptor>()
            .ok_or_else(|| HostError::Execution("creator environment is unavailable".into()))?;
        if !scope
            .get_slot::<AgentEnvironments>()
            .is_some_and(|environments| environments.is_live(&creator_environment))
            || creator
                .get_slot::<V8BindingState>()
                .is_some_and(|state| state.execution_retired())
        {
            return Err(HostError::Execution(
                "cannot create a child from a retired creator".into(),
            ));
        }
        let policy = creator.get_slot::<DocumentCodePolicy>();
        let inherited_base = if let Some(state) = creator.get_slot::<V8BindingState>() {
            let creator_scope = &mut v8::ContextScope::new(scope, creator);
            Some(
                oryn_js::v8_bindings::document_base_url(creator_scope, &state, true)
                    .map_err(|error| HostError::Execution(error.to_string()))?,
            )
        } else {
            None
        };
        let (realm, context) =
            create_native_realm_context(scope, origin, document_generation, frame_id, None)?;
        let local = v8::Local::new(scope, &context);
        if let Err(error) = install_native_document(
            scope,
            local,
            dom,
            realm,
            document_generation,
            document,
            policy.as_deref(),
        ) {
            if let Some(state) = local.get_slot::<V8BindingState>() {
                state.retire_execution();
            }
            if let Some(environment) = local.get_slot::<EnvironmentDescriptor>() {
                retire_environment(scope, environment.id);
            }
            let identities = scope.get_slot_mut::<NativeRealmIdentities>().unwrap();
            identities.pending_contexts.remove(&realm);
            identities.realms.remove(realm);
            return Err(error);
        }
        local
            .get_slot::<V8BindingState>()
            .expect("successful document installation has binding state")
            .set_inherited_base_url(inherited_base);
        Ok((realm, context))
    }

    impl RawV8Host {
        /// Run the agent's pending microtasks without executing a synthetic script
        /// or polling platform tasks. The parser calls this at script end tags.
        pub fn run_microtask_checkpoint(&mut self) -> Result<(), HostError> {
            let deadline = ExecutionDeadline::arm(&self.isolate, self.execution_wall_limit);
            v8::scope!(let handle_scope, &mut self.isolate);
            let context = v8::Local::new(
                handle_scope,
                self.contexts
                    .get(&self.active_realm)
                    .expect("active realm context exists"),
            );
            let scope = &mut v8::ContextScope::new(handle_scope, context);
            v8::tc_scope!(let scope, scope);
            queue_mutation_observer_microtask(scope);
            scope.perform_microtask_checkpoint();
            if deadline.expired() || scope.is_execution_terminating() {
                scope.cancel_terminate_execution();
                reset_mutation_observer_microtask_after_termination(scope);
                return Err(HostError::LimitExceeded {
                    resource: "wall time".into(),
                });
            }
            if scope.has_caught() {
                let exception = scope.exception();
                let message = scope.message();
                let detail = message
                    .map(|message| message.get(scope).to_rust_string_lossy(scope))
                    .unwrap_or_else(|| "microtask checkpoint failed".into());
                scope.reset();
                if let Some(exception) = exception {
                    let canceled = oryn_js::v8_bindings::report_script_exception(
                        scope, exception, message, false,
                    )
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                    if !canceled {
                        return Err(HostError::Execution(detail));
                    }
                }
            }
            if deadline.expired() || scope.is_execution_terminating() {
                scope.cancel_terminate_execution();
                reset_mutation_observer_microtask_after_termination(scope);
                return Err(HostError::LimitExceeded {
                    resource: "wall time".into(),
                });
            }
            Ok(())
        }

        /// Execute a parser-owned classic script and report uncaught exceptions to its realm.
        /// API execution continues to use `JavaScriptHost::execute` and propagate errors.
        pub fn execute_parser_script(
            &mut self,
            source: &str,
            resource_name: &str,
            muted_errors: bool,
        ) -> Result<(), HostError> {
            let deadline = ExecutionDeadline::arm(&self.isolate, self.execution_wall_limit);
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
            let name = v8::String::new(scope, resource_name)
                .ok_or_else(|| HostError::Execution("script name allocation failed".into()))?;
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
                false,
                None,
            );
            queue_mutation_observer_microtask(scope);
            let completed = v8::Script::compile(scope, source, Some(&origin))
                .and_then(|script| script.run(scope))
                .is_some();
            if deadline.expired() || scope.is_execution_terminating() {
                scope.cancel_terminate_execution();
                reset_mutation_observer_microtask_after_termination(scope);
                return Err(HostError::LimitExceeded {
                    resource: "wall time".into(),
                });
            }
            let mut result = Ok(());
            if !completed {
                // Preserve even thrown null/undefined, and capture source metadata before reset.
                let exception = scope.exception();
                let message = scope.message();
                let detail = message
                    .map(|message| message.get(scope).to_rust_string_lossy(scope))
                    .unwrap_or_else(|| "script execution failed".into());
                scope.reset();
                let canceled = if let Some(exception) = exception {
                    oryn_js::v8_bindings::report_script_exception(
                        scope,
                        exception,
                        message,
                        muted_errors,
                    )
                    .map_err(|error| HostError::Execution(error.to_string()))?
                } else {
                    false
                };
                if !canceled {
                    result = Err(HostError::Execution(detail));
                }
            }
            // Error reporting is synchronous; pending jobs still run after a failed script.
            if !deadline.expired() && !scope.is_execution_terminating() {
                queue_mutation_observer_microtask(scope);
                scope.perform_microtask_checkpoint();
            }
            if deadline.expired() || scope.is_execution_terminating() {
                scope.cancel_terminate_execution();
                reset_mutation_observer_microtask_after_termination(scope);
                return Err(HostError::LimitExceeded {
                    resource: "wall time".into(),
                });
            }
            if scope.has_caught() {
                let exception = scope.exception();
                let message = scope.message();
                let detail = message
                    .map(|message| message.get(scope).to_rust_string_lossy(scope))
                    .unwrap_or_else(|| "microtask checkpoint failed".into());
                scope.reset();
                if let Some(exception) = exception {
                    let canceled = oryn_js::v8_bindings::report_script_exception(
                        scope,
                        exception,
                        message,
                        muted_errors,
                    )
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                    if !canceled {
                        result = Err(HostError::Execution(detail));
                    }
                }
            }
            if deadline.expired() || scope.is_execution_terminating() {
                scope.cancel_terminate_execution();
                reset_mutation_observer_microtask_after_termination(scope);
                return Err(HostError::LimitExceeded {
                    resource: "wall time".into(),
                });
            }
            result
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
            let watchdog = ExecutionWatchdog::new(&mut isolate);
            isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
            isolate.set_host_import_module_dynamically_callback(dynamic_import);
            isolate.set_host_initialize_import_meta_object_callback(initialize_import_meta);
            isolate.set_slot(PromiseRejections::default());
            isolate.set_slot(ModuleErrorReports::default());
            isolate.set_slot(AsyncFetches::default());
            isolate.set_slot(DedicatedWorkers::default());
            isolate.set_slot(TimerRegistry::default());
            isolate.set_slot(AgentEnvironments::default());
            isolate.set_slot(FrameLifetimes::default());
            isolate.set_slot(oryn_js::v8_bindings::DomLifecycleCheckpoint(
                checkpoint_frame_lifetimes,
            ));
            isolate.set_slot(ModuleNames::default());
            isolate.set_slot(ModuleCache::default());
            isolate.set_slot(crate::automation_trace::AutomationTrace::default());
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
            isolate.set_slot(NativeRealmIdentities::default());
            let (active_realm, context) = {
                v8::scope!(let scope, &mut isolate);
                create_native_realm_context(scope, origin.into(), 1, None, None)
                    .expect("create initial execution realm")
            };
            let mut contexts = BTreeMap::new();
            contexts.insert(active_realm, context);
            isolate
                .get_slot_mut::<NativeRealmIdentities>()
                .unwrap()
                .pending_contexts
                .remove(&active_realm);
            Self {
                watchdog,
                isolate,
                contexts,
                active_realm,
                binding_states: BTreeMap::new(),
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
            let document = dom.borrow().document();
            self.install_dom_root(dom, document_generation, document)
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
            let realm = self.active_realm;
            let context = self
                .contexts
                .get(&realm)
                .ok_or_else(|| HostError::Execution("active realm context is unavailable".into()))?
                .clone();
            v8::scope!(let handle_scope, &mut self.isolate);
            let context = v8::Local::new(handle_scope, &context);
            let policy = context.get_slot::<DocumentCodePolicy>();
            let state = install_native_document(
                handle_scope,
                context,
                dom,
                realm,
                document_generation,
                document,
                policy.as_deref(),
            )?;
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

        pub fn schedule_dynamic_frame(&mut self, frame: NodeId) -> Result<(), HostError> {
            self.binding_states
                .get(&self.active_realm)
                .ok_or_else(|| HostError::Execution("active Rust DOM is unavailable".into()))?
                .schedule_dynamic_frame(frame);
            Ok(())
        }

        pub fn reschedule_dynamic_frame(&mut self, frame: NodeId) -> Result<(), HostError> {
            self.binding_states
                .get(&self.active_realm)
                .ok_or_else(|| HostError::Execution("active Rust DOM is unavailable".into()))?
                .reschedule_dynamic_frame(frame);
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
            // Browsing-context ancestry is native state. Reading window.top
            // here would let an author getter execute during linking or replace
            // the destination with an unrelated object.
            let mut top_realm = self.active_realm;
            let mut visited = BTreeSet::new();
            while visited.insert(top_realm) {
                let parent = self
                    .isolate
                    .get_slot::<FrameLifetimes>()
                    .unwrap()
                    .live
                    .values()
                    .find(|entry| entry.environment.get().realm == top_realm)
                    .map(|entry| entry.parent_environment.realm);
                let Some(parent) = parent else { break };
                top_realm = parent;
            }
            if self
                .isolate
                .get_slot::<FrameLifetimes>()
                .unwrap()
                .live
                .values()
                .any(|entry| entry.environment.get().realm == top_realm)
            {
                return Err(HostError::Execution("cyclic native frame ancestry".into()));
            }
            let top_context = self
                .contexts
                .get(&top_realm)
                .cloned()
                .ok_or_else(|| HostError::Execution("top frame context unavailable".into()))?;
            let active = self
                .binding_states
                .get(&self.active_realm)
                .cloned()
                .ok_or_else(|| HostError::Execution("active Rust DOM is unavailable".into()))?;
            let parent_origin = self
                .isolate
                .get_slot::<NativeRealmIdentities>()
                .unwrap()
                .realms
                .get(self.active_realm)
                .map(|descriptor| descriptor.origin.clone())
                .ok_or_else(|| {
                    HostError::Execution("parent realm descriptor is unavailable".into())
                })?;
            let child_origin = self
                .isolate
                .get_slot::<NativeRealmIdentities>()
                .unwrap()
                .realms
                .get(realm)
                .map(|descriptor| descriptor.origin.clone())
                .ok_or_else(|| {
                    HostError::Execution("frame realm descriptor is unavailable".into())
                })?;
            let same_origin = document.is_some() && parent_origin == child_origin;
            let parent_origin = parent_origin.serialize();
            let child_origin = child_origin.serialize();
            v8::scope!(let handle_scope, &mut self.isolate);
            let parent_context_local = v8::Local::new(handle_scope, &parent_context);
            let child_context_local = v8::Local::new(handle_scope, &child_context);
            bind_native_frame_lifetime(
                handle_scope,
                parent_context_local,
                child_context_local,
                frame,
            )?;
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
            let parent_top = {
                let top_context = v8::Local::new(handle_scope, &top_context);
                v8::Global::new(handle_scope, top_context.global(handle_scope))
            };
            let frame_element = if same_origin {
                let parent_context = v8::Local::new(handle_scope, &parent_context);
                let scope = &mut v8::ContextScope::new(handle_scope, parent_context);
                let wrapper = wrap_node(scope, &active, frame)
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                Some(v8::Global::new(scope, wrapper))
            } else {
                None
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
                let child_context = v8::Local::new(scope, &child_context);
                install_frame_closed_accessor(scope, proxy, child_context)?;
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
                install_frame_closed_accessor(scope, child_window, child_context)?;
                let child_proxy = v8::Local::new(scope, &public_child_proxy);
                let parent_proxy = v8::Local::new(scope, &parent_proxy);
                let parent_top = v8::Local::new(scope, &parent_top);
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
                let parent_key = v8::String::new(scope, "parent")
                    .ok_or_else(|| HostError::Execution("allocate frame parent".into()))?;
                child_window.set(scope, parent_key.into(), parent_proxy.into());
                // `top` is transitive. Pointing it at the immediate parent
                // causes messages from a grandchild to terminate in the
                // intermediate iframe instead of the top-level harness.
                oryn_js::v8_bindings::set_native_window_top(scope, child_window, parent_top)
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                let frame_element = frame_element
                    .as_ref()
                    .map(|element| v8::Local::new(scope, element));
                oryn_js::v8_bindings::set_native_window_frame_element(
                    scope,
                    child_window,
                    frame_element,
                )
                .map_err(|error| HostError::Execution(error.to_string()))?;
            }
            if let Some(document) = document {
                active.set_frame_document(frame, document);
                active.set_frame_committed_window(frame, child_window);
            } else {
                active.clear_frame_document(frame);
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

        pub fn commit_location(&self, href: &str) -> Result<(), HostError> {
            self.binding_states
                .get(&self.active_realm)
                .ok_or_else(|| HostError::Execution("active Rust DOM is unavailable".into()))?
                .commit_location(href)
                .map_err(|error| HostError::Execution(error.to_string()))
        }

        pub fn take_location_navigation(&self) -> Option<oryn_js::v8_bindings::LocationNavigation> {
            self.binding_states
                .get(&self.active_realm)
                .and_then(|state| state.take_location_navigation())
        }

        pub fn set_forms_allowed(&self, allowed: bool) -> Result<(), HostError> {
            self.binding_states
                .get(&self.active_realm)
                .ok_or_else(|| HostError::Execution("active DOM is unavailable".into()))?
                .set_forms_allowed(allowed);
            Ok(())
        }

        pub fn prepare_document_images(&mut self) -> Result<(), HostError> {
            let state = self
                .binding_states
                .get(&self.active_realm)
                .cloned()
                .ok_or_else(|| {
                    HostError::Execution("active image document is unavailable".into())
                })?;
            let context = self
                .contexts
                .get(&self.active_realm)
                .cloned()
                .ok_or_else(|| {
                    HostError::Execution("active image context is unavailable".into())
                })?;
            let _deadline = ExecutionDeadline::arm(&self.isolate, self.execution_wall_limit);
            v8::scope!(let handle_scope, &mut self.isolate);
            let context = v8::Local::new(handle_scope, &context);
            let scope = &mut v8::ContextScope::new(handle_scope, context);
            oryn_js::v8_bindings::prepare_document_images(scope, &state)
                .map_err(|error| HostError::Execution(error.to_string()))
        }

        pub fn realm_has_pending_images(&self, realm: RealmId) -> bool {
            self.binding_states
                .get(&realm)
                .is_some_and(|state| state.has_pending_images())
        }

        pub fn schedule_navigation(
            &self,
            request: oryn_js::v8_bindings::LocationNavigation,
        ) -> Result<(), HostError> {
            let state = self
                .binding_states
                .get(&self.active_realm)
                .ok_or_else(|| HostError::Execution("active DOM is unavailable".into()))?;
            state
                .commit_location(&request.url)
                .map_err(|error| HostError::Execution(error.to_string()))?;
            state.schedule_navigation(request);
            Ok(())
        }

        pub fn take_history_url_update(&self) -> Option<String> {
            self.binding_states
                .get(&self.active_realm)
                .and_then(|state| state.take_history_url_update())
        }

        pub fn configure_trusted_types(
            &mut self,
            config: oryn_js::trusted_types::TrustedTypesConfig,
            allow_string_code_generation: bool,
        ) -> Result<(), HostError> {
            let realm = self.active_realm;
            self.binding_states
                .get(&realm)
                .ok_or_else(|| HostError::Execution("active Rust DOM is unavailable".into()))?
                .configure_trusted_types(config.clone());
            let context = self
                .contexts
                .get(&realm)
                .ok_or_else(|| HostError::Execution("active realm context is unavailable".into()))?
                .clone();
            v8::scope!(let handle_scope, &mut self.isolate);
            let context = v8::Local::new(handle_scope, &context);
            context.set_allow_generation_from_strings(allow_string_code_generation);
            context.set_slot(Rc::new(DocumentCodePolicy {
                trusted_types: config,
                allow_eval: allow_string_code_generation,
            }));
            Ok(())
        }

        pub fn active_dom(&self) -> Option<SharedDom> {
            self.binding_states
                .get(&self.active_realm)
                .map(|state| state.dom())
        }

        pub(crate) fn validate_primary_arena(&self, dom: &SharedDom) -> Result<(), HostError> {
            if !self
                .binding_states
                .get(&RealmId(0))
                .is_some_and(|state| Rc::ptr_eq(&state.dom(), dom))
            {
                return Err(HostError::Execution(
                    "window agent primary document arena mismatch".into(),
                ));
            }
            Ok(())
        }

        pub fn take_pending_dynamic_scripts(&self) -> Result<Vec<NodeId>, HostError> {
            self.binding_states
                .get(&self.active_realm)
                .ok_or_else(|| HostError::Execution("active Rust DOM is unavailable".into()))
                .map(|state| state.take_pending_dynamic_scripts())
        }

        pub fn take_all_pending_dynamic_scripts(
            &self,
        ) -> Result<Vec<(RealmId, NodeId)>, HostError> {
            Ok(self
                .binding_states
                .iter()
                .flat_map(|(realm, state)| {
                    state
                        .take_pending_dynamic_scripts()
                        .into_iter()
                        .map(|script| (*realm, script))
                })
                .collect())
        }

        pub fn take_all_pending_dynamic_stylesheets(
            &self,
        ) -> Result<Vec<(RealmId, NodeId)>, HostError> {
            Ok(self
                .binding_states
                .iter()
                .flat_map(|(realm, state)| {
                    state
                        .take_pending_dynamic_stylesheets()
                        .into_iter()
                        .map(|stylesheet| (*realm, stylesheet))
                })
                .collect())
        }

        pub fn take_all_pending_referrer_policies(
            &self,
        ) -> Result<Vec<(RealmId, String)>, HostError> {
            Ok(self
                .binding_states
                .iter()
                .flat_map(|(realm, state)| {
                    state
                        .take_pending_referrer_policies()
                        .into_iter()
                        .map(|policy| (*realm, policy))
                })
                .collect())
        }

        pub fn take_all_pending_dynamic_frames(&self) -> Result<Vec<(RealmId, NodeId)>, HostError> {
            Ok(self
                .binding_states
                .iter()
                .flat_map(|(realm, state)| {
                    state
                        .take_pending_dynamic_frames()
                        .into_iter()
                        .map(|frame| (*realm, frame))
                })
                .collect())
        }

        pub fn take_pending_default_actions(
            &self,
        ) -> Result<Vec<(RealmId, PendingDefaultAction)>, HostError> {
            Ok(self
                .binding_states
                .iter()
                .flat_map(|(realm, state)| {
                    state
                        .take_pending_default_actions()
                        .into_iter()
                        .map(|action| (*realm, action))
                })
                .collect())
        }

        pub fn install_layout(&mut self, layout: BindingLayout) -> Result<(), HostError> {
            self.binding_states
                .get(&self.active_realm)
                .ok_or_else(|| HostError::Execution("active Rust DOM is unavailable".into()))?
                .set_layout(layout);
            Ok(())
        }

        pub fn set_inherited_base_url(&mut self, base: Option<String>) -> Result<(), HostError> {
            self.binding_states
                .get(&self.active_realm)
                .ok_or_else(|| HostError::Execution("active Rust DOM is unavailable".into()))?
                .set_inherited_base_url(base);
            Ok(())
        }

        pub fn install_layout_provider(
            &mut self,
            provider: Rc<dyn BindingLayoutProvider>,
        ) -> Result<(), HostError> {
            let realm = self.active_realm;
            let state = self
                .binding_states
                .get(&realm)
                .cloned()
                .ok_or_else(|| HostError::Execution("active Rust DOM is unavailable".into()))?;
            state.set_layout_provider(provider);
            let context = self
                .contexts
                .get(&realm)
                .cloned()
                .ok_or_else(|| HostError::Execution("active realm is unavailable".into()))?;
            v8::scope!(let handle_scope, &mut self.isolate);
            let context = v8::Local::new(handle_scope, &context);
            let scope = &mut v8::ContextScope::new(handle_scope, context);
            oryn_js::v8_bindings::install_css_property_accessors(scope, &state)
                .map_err(|error| HostError::Execution(error.to_string()))
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

        /// Invoke native form validation/submission without consulting page globals.
        pub fn request_submit(&mut self, form: NodeId) -> Result<(), HostError> {
            let state = self
                .binding_states
                .get(&self.active_realm)
                .cloned()
                .ok_or_else(|| HostError::Execution("active DOM is unavailable".into()))?;
            let context = self
                .contexts
                .get(&self.active_realm)
                .cloned()
                .ok_or_else(|| HostError::Execution("active realm is unavailable".into()))?;
            let deadline = ExecutionDeadline::arm(&self.isolate, self.execution_wall_limit);
            v8::scope!(let handle_scope, &mut self.isolate);
            let context = v8::Local::new(handle_scope, &context);
            let scope = &mut v8::ContextScope::new(handle_scope, context);
            v8::tc_scope!(let scope, scope);
            let result =
                oryn_js::v8_bindings::submissions::request_submit(scope, &state, form, None);
            if deadline.expired() || scope.is_execution_terminating() {
                scope.cancel_terminate_execution();
                reset_mutation_observer_microtask_after_termination(scope);
                return Err(HostError::LimitExceeded {
                    resource: "wall time".into(),
                });
            }
            result.map_err(|error| HostError::Execution(error.to_string()))
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

        /// Drain native custom-element reactions under the page's execution
        /// deadline, independently of author-writable JavaScript globals.
        pub fn run_custom_element_reactions(&mut self) -> Result<(), HostError> {
            let Some(state) = self.binding_states.get(&self.active_realm()).cloned() else {
                return Ok(());
            };
            if !state.has_pending_custom_element_reactions() {
                return Ok(());
            }
            let context = self
                .contexts
                .get(&self.active_realm())
                .cloned()
                .ok_or_else(|| HostError::Execution("custom element realm missing".into()))?;
            let deadline = ExecutionDeadline::arm(&self.isolate, self.execution_wall_limit);
            v8::scope!(let handle_scope, &mut self.isolate);
            let context = v8::Local::new(handle_scope, &context);
            let scope = &mut v8::ContextScope::new(handle_scope, context);
            let result = state
                .finish_custom_element_reactions(scope)
                .map_err(|error| HostError::Execution(error.to_string()));
            if deadline.expired() || scope.is_execution_terminating() {
                scope.cancel_terminate_execution();
                return Err(HostError::LimitExceeded {
                    resource: "wall time".into(),
                });
            }
            result
        }

        pub fn run_dom_observer_checkpoint(&mut self) -> Result<usize, HostError> {
            let Some(dom) = self.active_dom() else {
                return Ok(0);
            };
            let initial_mutations = dom.borrow_mut().drain_mutations();
            // Most rendering checkpoints have no notifications. Do not spawn
            // a deadline thread or enter V8 on that common read-only path.
            if initial_mutations.is_empty()
                && !has_pending_mutation_observer_microtask(&self.isolate)
                && !self
                    .binding_states
                    .values()
                    .any(|state| state.has_pending_selection_change())
            {
                return Ok(0);
            }
            let mut initial_mutations = Some(initial_mutations);
            let _execution_deadline =
                ExecutionDeadline::arm(&self.isolate, self.execution_wall_limit);
            let mut delivered = 0_usize;
            for _ in 0..1024 {
                let Some(dom) = self.active_dom() else {
                    return Ok(delivered);
                };
                let mutations = initial_mutations
                    .take()
                    .unwrap_or_else(|| dom.borrow_mut().drain_mutations());
                let cursor = dom.borrow().mutation_cursor();
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
                    if _execution_deadline.expired() || scope.is_execution_terminating() {
                        scope.cancel_terminate_execution();
                        reset_mutation_observer_microtask_after_termination(scope);
                        return Err(HostError::LimitExceeded {
                            resource: "wall time".into(),
                        });
                    }
                    delivered += usize::from(
                        fixup_focus_eligibility(scope, &state)
                            .map_err(|error| HostError::Execution(error.to_string()))?,
                    );
                    delivered += usize::from(
                        deliver_selection_change(scope, &state)
                            .map_err(|error| HostError::Execution(error.to_string()))?,
                    );
                    queue_mutation_observer_microtask(scope);
                    scope.perform_microtask_checkpoint();
                    if _execution_deadline.expired() || scope.is_execution_terminating() {
                        scope.cancel_terminate_execution();
                        reset_mutation_observer_microtask_after_termination(scope);
                        return Err(HostError::LimitExceeded {
                            resource: "wall time".into(),
                        });
                    }
                }
                dom.borrow_mut()
                    .release_mutation_roots(&mutations)
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                if dom.borrow().mutation_cursor() == cursor {
                    return Ok(delivered);
                }
            }
            Err(HostError::LimitExceeded {
                resource: "mutation observer checkpoint iterations".into(),
            })
        }

        pub fn next_timer_delay(&mut self) -> Option<Duration> {
            if self
                .binding_states
                .values()
                .any(|state| state.has_pending_control_event())
            {
                return Some(Duration::ZERO);
            }
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
                let control_state = self
                    .binding_states
                    .iter()
                    .find(|(_, state)| state.has_pending_control_event())
                    .map(|(realm, state)| (*realm, state.clone()));
                if let Some((realm, state)) = control_state {
                    let Some(context) = self.contexts.get(&realm).cloned() else {
                        break;
                    };
                    let deadline = ExecutionDeadline::arm(&self.isolate, self.execution_wall_limit);
                    v8::scope!(let handle_scope, &mut self.isolate);
                    let context = v8::Local::new(handle_scope, &context);
                    let scope = &mut v8::ContextScope::new(handle_scope, context);
                    deliver_control_event(scope, &state)
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                    queue_mutation_observer_microtask(scope);
                    scope.perform_microtask_checkpoint();
                    if deadline.expired() || scope.is_execution_terminating() {
                        scope.cancel_terminate_execution();
                        reset_mutation_observer_microtask_after_termination(scope);
                        return Err(HostError::LimitExceeded {
                            resource: "control event task wall time".into(),
                        });
                    }
                    delivered += 1;
                    continue;
                }
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
                            timer.nesting_level,
                        )
                    });
                let Some((context, callback, arguments, repeating, nesting_level)) = timer else {
                    continue;
                };
                if !repeating && let Some(registry) = self.isolate.get_slot_mut::<TimerRegistry>() {
                    registry.callbacks.remove(&timer_id);
                }
                {
                    let nesting_state = self
                        .isolate
                        .get_slot::<TimerRegistry>()
                        .expect("timer registry")
                        .task_nesting
                        .clone();
                    let nesting = TimerTaskNesting::enter(nesting_state, nesting_level);
                    let _deadline =
                        ExecutionDeadline::arm(&self.isolate, self.execution_wall_limit);
                    v8::scope!(let handle_scope, &mut self.isolate);
                    let context = v8::Local::new(handle_scope, &context);
                    let scope = &mut v8::ContextScope::new(handle_scope, context);
                    queue_mutation_observer_microtask(scope);
                    v8::tc_scope!(let scope, scope);
                    let callback = v8::Local::new(scope, &callback);
                    let arguments = arguments
                        .iter()
                        .map(|value| v8::Local::new(scope, value))
                        .collect::<Vec<_>>();
                    let receiver = context.global(scope).into();
                    if callback.call(scope, receiver, &arguments).is_none() {
                        if _deadline.expired() || scope.is_execution_terminating() {
                            scope.cancel_terminate_execution();
                            reset_mutation_observer_microtask_after_termination(scope);
                            if let Some(registry) = scope.get_slot_mut::<TimerRegistry>()
                                && let Some(timer) = registry.callbacks.remove(&timer_id)
                            {
                                registry.scheduler.cancel_timer(timer.scheduled_id);
                            }
                            return Err(HostError::LimitExceeded {
                                resource: "timer callback wall time".into(),
                            });
                        }
                        let exception = scope.exception();
                        let message = scope.message();
                        scope.reset();
                        // The callback's realm, not the timer-owning Window
                        // or the thrown object's realm, receives ErrorEvent.
                        // Ordinary script errors do not terminate the event loop.
                        if let Some(exception) = exception {
                            report_timer_exception(scope, callback, exception, message)?;
                        }
                    }
                    // Microtasks are not timer tasks and must not inherit the
                    // callback's nesting level. RAII also restores it on errors.
                    drop(nesting);
                    queue_mutation_observer_microtask(scope);
                    scope.perform_microtask_checkpoint();
                    if _deadline.expired() || scope.is_execution_terminating() {
                        scope.cancel_terminate_execution();
                        reset_mutation_observer_microtask_after_termination(scope);
                        if let Some(registry) = scope.get_slot_mut::<TimerRegistry>()
                            && let Some(timer) = registry.callbacks.remove(&timer_id)
                        {
                            registry.scheduler.cancel_timer(timer.scheduled_id);
                        }
                        return Err(HostError::LimitExceeded {
                            resource: "wall time".into(),
                        });
                    }
                }
                if repeating && let Some(registry) = self.isolate.get_slot_mut::<TimerRegistry>() {
                    registry
                        .scheduler
                        .advance_to(registry.started.elapsed().as_millis() as u64);
                    if let Some(timer) = registry.callbacks.get_mut(&timer_id) {
                        let environment = timer.environment;
                        timer.scheduled_id = registry.scheduler.set_timeout_with(
                            nested_timer_delay(timer.delay_ms, timer.nesting_level),
                            |_| Task {
                                source: TaskSource::Timer,
                                realm: environment.realm,
                                document_generation: environment.document_generation,
                                action_id: None,
                                payload: timer_id,
                            },
                        );
                        timer.nesting_level = timer.nesting_level.saturating_add(1);
                    }
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
                        fetches.receiver.try_recv().ok()
                    } else if wait {
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
                if self
                    .isolate
                    .get_slot_mut::<AsyncFetches>()
                    .unwrap()
                    .retired
                    .remove(&completion.operation_id)
                {
                    continue;
                }
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
                let _execution_deadline = ExecutionDeadline::arm(
                    &self.isolate,
                    deadline.saturating_duration_since(Instant::now()),
                );
                v8::scope!(let handle_scope, &mut self.isolate);
                let context = v8::Local::new(handle_scope, &pending.context);
                let scope = &mut v8::ContextScope::new(handle_scope, context);
                queue_mutation_observer_microtask(scope);
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
                queue_mutation_observer_microtask(scope);
                scope.perform_microtask_checkpoint();
                if _execution_deadline.expired() || scope.is_execution_terminating() {
                    scope.cancel_terminate_execution();
                    reset_mutation_observer_microtask_after_termination(scope);
                    return Err(HostError::LimitExceeded {
                        resource: "wall time".into(),
                    });
                }
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
                let mut scheduled = self
                    .binding_states
                    .iter()
                    .filter_map(|(realm, state)| {
                        self.contexts
                            .get(realm)
                            .cloned()
                            .map(|context| (context, state.clone()))
                    })
                    .collect::<Vec<_>>();
                scheduled.extend(
                    self.isolate
                        .get_slot::<DedicatedWorkers>()
                        .into_iter()
                        .flat_map(|registry| registry.workers.values())
                        .filter(|worker| !worker.closed)
                        .filter_map(|worker| {
                            worker
                                .binding_state
                                .as_ref()
                                .cloned()
                                .map(|state| (worker.context.clone(), state))
                        }),
                );
                for (context, state) in scheduled {
                    let _execution_deadline = ExecutionDeadline::arm(
                        &self.isolate,
                        deadline.saturating_duration_since(Instant::now()),
                    );
                    v8::scope!(let handle_scope, &mut self.isolate);
                    let context = v8::Local::new(handle_scope, &context);
                    let scope = &mut v8::ContextScope::new(handle_scope, context);
                    queue_mutation_observer_microtask(scope);
                    // Each platform completion is a task; its promise jobs
                    // must run before the next completion is delivered.
                    pass += state
                        .drain_platform_completions(scope, 1)
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                    if pass > 0 {
                        queue_mutation_observer_microtask(scope);
                        scope.perform_microtask_checkpoint();
                        if _execution_deadline.expired() || scope.is_execution_terminating() {
                            scope.cancel_terminate_execution();
                            reset_mutation_observer_microtask_after_termination(scope);
                            return Err(HostError::LimitExceeded {
                                resource: "wall time".into(),
                            });
                        }
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
                    .any(|state| state.has_pending_non_image_work())
                    || self
                        .isolate
                        .get_slot::<DedicatedWorkers>()
                        .is_some_and(|registry| {
                            registry.workers.values().any(|worker| {
                                !worker.closed
                                    && worker
                                        .binding_state
                                        .as_ref()
                                        .is_some_and(|state| state.has_pending_platform_work())
                            })
                        });
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
                || self
                    .isolate
                    .get_slot::<DedicatedWorkers>()
                    .is_some_and(|registry| {
                        registry.workers.values().any(|worker| {
                            !worker.closed
                                && worker
                                    .binding_state
                                    .as_ref()
                                    .is_some_and(|state| state.has_pending_platform_work())
                        })
                    })
        }

        pub fn has_runnable_page_work(&mut self) -> bool {
            self.has_pending_async_fetches()
                || self.next_timer_delay().is_some_and(|delay| delay.is_zero())
                || self
                    .binding_states
                    .values()
                    .any(|state| state.has_pending_platform_work())
                || self
                    .isolate
                    .get_slot::<DedicatedWorkers>()
                    .is_some_and(|registry| {
                        registry.workers.values().any(|worker| {
                            !worker.closed
                                && worker
                                    .binding_state
                                    .as_ref()
                                    .is_some_and(|state| state.has_pending_platform_work())
                        })
                    })
        }

        /// Create another global realm in this page's isolate. DOM wrappers are
        /// still installed separately; this method only establishes the V8
        /// context and its immutable security identity.
        pub fn create_realm(
            &mut self,
            origin: impl Into<Origin>,
            document_generation: u64,
            frame_id: Option<u64>,
        ) -> Result<RealmId, HostError> {
            self.create_realm_with_global(origin.into(), document_generation, frame_id, None)
        }

        fn create_realm_with_global(
            &mut self,
            origin: Origin,
            document_generation: u64,
            frame_id: Option<u64>,
            global: Option<v8::Global<v8::Object>>,
        ) -> Result<RealmId, HostError> {
            v8::scope!(let scope, &mut self.isolate);
            // Preserve the legacy inherited-policy behavior, but select its
            // actual creator explicitly. Navigation response policy selection
            // is a separate document-loader responsibility.
            let policy = self.contexts.get(&self.active_realm).and_then(|context| {
                v8::Local::new(scope, context).get_slot::<DocumentCodePolicy>()
            });
            let (realm, context) =
                create_native_realm_context(scope, origin, document_generation, frame_id, global)?;
            if let Some(policy) = policy {
                v8::Local::new(scope, &context).set_slot(policy);
            }
            self.contexts.insert(realm, context);
            scope
                .get_slot_mut::<NativeRealmIdentities>()
                .unwrap()
                .pending_contexts
                .remove(&realm);
            Ok(realm)
        }

        /// Create a fresh frame document realm while retaining the browsing
        /// context's native global proxy. This is for ordinary navigation, not
        /// the initial same-origin about:blank Window reuse case.
        pub fn replace_frame_realm(
            &mut self,
            previous: RealmId,
            origin: impl Into<Origin>,
            document_generation: u64,
        ) -> Result<RealmId, HostError> {
            let frame_id = self
                .isolate
                .get_slot::<NativeRealmIdentities>()
                .unwrap()
                .realms
                .get(previous)
                .and_then(|descriptor| descriptor.frame_id)
                .ok_or_else(|| {
                    HostError::Execution("replacement requires an existing frame realm".into())
                })?;
            // Exhaustion must not detach the still-live old WindowProxy.
            self.isolate
                .get_slot::<NativeRealmIdentities>()
                .unwrap()
                .realms
                .ensure_capacity()
                .map_err(|_| HostError::LimitExceeded {
                    resource: "realm identities".into(),
                })?;
            let context = self
                .contexts
                .get(&previous)
                .cloned()
                .ok_or_else(|| HostError::Execution("frame context is unavailable".into()))?;
            let global = {
                v8::scope!(let scope, &mut self.isolate);
                let context = v8::Local::new(scope, &context);
                let global = v8::Global::new(scope, context.global(scope));
                oryn_js::v8_context_bridge::detach_global(scope, context);
                global
            };
            let was_active = self.active_realm == previous;
            let replacement = self.create_realm_with_global(
                origin.into(),
                document_generation,
                Some(frame_id),
                Some(global),
            )?;
            self.remove_realm(previous)?;
            if was_active {
                self.active_realm = replacement;
            }
            Ok(replacement)
        }

        /// The incumbent realm belongs to the calling script, which can differ
        /// from the native function's creation realm and the last entered realm.
        pub fn incumbent_realm(scope: &mut v8::PinScope) -> Option<Rc<RealmDescriptor>> {
            oryn_js::v8_context_bridge::incumbent_slot(scope)
        }

        pub fn create_realm_with_dom(
            &mut self,
            origin: impl Into<Origin>,
            document_generation: u64,
            frame_id: Option<u64>,
            dom: Rc<RefCell<oryn_dom::DomRuntime>>,
        ) -> Result<RealmId, HostError> {
            let document = dom.borrow().document();
            self.create_realm_with_dom_root(origin, document_generation, frame_id, dom, document)
        }

        pub fn create_realm_with_dom_root(
            &mut self,
            origin: impl Into<Origin>,
            document_generation: u64,
            frame_id: Option<u64>,
            dom: Rc<RefCell<oryn_dom::DomRuntime>>,
            document: NodeId,
        ) -> Result<RealmId, HostError> {
            let creator = self
                .contexts
                .get(&self.active_realm)
                .cloned()
                .ok_or_else(|| HostError::Execution("creator realm is unavailable".into()))?;
            let realm = {
                v8::scope!(let scope, &mut self.isolate);
                let creator = v8::Local::new(scope, &creator);
                create_native_document_context(
                    scope,
                    creator,
                    origin.into(),
                    document_generation,
                    frame_id,
                    dom,
                    document,
                )?
                .0
            };
            self.adopt_created_realms();
            Ok(realm)
        }

        pub fn activate_realm(&mut self, realm: RealmId) -> Result<(), HostError> {
            self.adopt_created_realms();
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

        pub fn realm_ids(&self) -> Vec<RealmId> {
            self.binding_states.keys().copied().collect()
        }

        /// Transfer callback-created contexts into normal driver ownership.
        /// Read the final binding slot, not a snapshot taken before bootstrap.
        /// Retired bindings are retained here until ordinary realm retirement
        /// releases them; adoption must never reactivate an environment.
        pub fn adopt_created_realms(&mut self) -> Vec<RealmId> {
            let pending = std::mem::take(
                &mut self
                    .isolate
                    .get_slot_mut::<NativeRealmIdentities>()
                    .unwrap()
                    .pending_contexts,
            );
            let mut adopted = Vec::with_capacity(pending.len());
            for (realm, context) in pending {
                if self.contexts.contains_key(&realm) {
                    continue;
                }
                let state = {
                    v8::scope!(let scope, &mut self.isolate);
                    v8::Local::new(scope, &context).get_slot::<V8BindingState>()
                };
                if let Some(state) = state {
                    self.binding_states.insert(realm, state);
                }
                self.contexts.insert(realm, context);
                adopted.push(realm);
            }
            adopted
        }

        pub fn realm_execution_live(&self, realm: RealmId) -> bool {
            self.binding_states
                .get(&realm)
                .is_some_and(|state| !state.execution_retired())
        }

        pub fn realm_for_document(&self, dom: &SharedDom, document: NodeId) -> Option<RealmId> {
            self.binding_states.iter().find_map(|(realm, state)| {
                (!state.execution_retired()
                    && state.document_root() == document
                    && Rc::ptr_eq(&state.dom(), dom))
                .then_some(*realm)
            })
        }

        pub fn take_discarded_frame_realms(&mut self) -> Vec<RealmId> {
            self.isolate
                .get_slot_mut::<FrameLifetimes>()
                .map(|registry| {
                    std::mem::take(&mut registry.discarded_realms)
                        .into_iter()
                        .collect()
                })
                .unwrap_or_default()
        }

        pub fn realm_descriptor(&self, realm: RealmId) -> Option<&RealmDescriptor> {
            self.isolate
                .get_slot::<NativeRealmIdentities>()?
                .realms
                .get(realm)
        }

        /// Replace or remove a frame realm after navigation. The top-level
        /// realm cannot be removed because callers always need an active
        /// execution context.
        pub fn remove_realm(&mut self, realm: RealmId) -> Result<RealmDescriptor, HostError> {
            self.adopt_created_realms();
            if self.contexts.len() == 1 || realm == RealmId(0) {
                return Err(HostError::Execution(
                    "the top-level JavaScript realm cannot be removed".into(),
                ));
            }
            let context = self.contexts.remove(&realm).ok_or_else(|| {
                HostError::Execution(format!("unknown JavaScript realm {}", realm.0))
            })?;
            let environment = {
                v8::scope!(let scope, &mut self.isolate);
                v8::Local::new(scope, &context)
                    .get_slot::<EnvironmentDescriptor>()
                    .map(|value| *value)
            };
            if let Some(environment) = environment {
                retire_environment(&mut self.isolate, environment.id);
            }
            if let Some(state) = self.binding_states.remove(&realm) {
                state.retire_execution();
            }
            if self.active_realm == realm {
                self.active_realm = RealmId(0);
            }
            self.isolate
                .get_slot_mut::<NativeRealmIdentities>()
                .unwrap()
                .realms
                .remove(realm)
                .ok_or_else(|| {
                    HostError::Execution(format!("missing descriptor for realm {}", realm.0))
                })
        }

        pub fn execute_module(
            &mut self,
            resource_name: &str,
            source: &str,
            dependencies: &BTreeMap<String, String>,
        ) -> Result<(), HostError> {
            self.execute_module_inner(resource_name, source, dependencies, false, true)
        }

        /// Run a document's module script, including errors after top-level await.
        pub fn execute_module_script(
            &mut self,
            resource_name: &str,
            source: &str,
            dependencies: &BTreeMap<String, String>,
            cache_root: bool,
        ) -> Result<(), HostError> {
            self.execute_module_inner(resource_name, source, dependencies, true, cache_root)
        }

        fn execute_module_inner(
            &mut self,
            resource_name: &str,
            source: &str,
            dependencies: &BTreeMap<String, String>,
            report_errors: bool,
            cache_root: bool,
        ) -> Result<(), HostError> {
            let deadline = ExecutionDeadline::arm(&self.isolate, self.execution_wall_limit);
            self.isolate.set_slot(ModuleSources(dependencies.clone()));
            v8::scope!(let handle_scope, &mut self.isolate);
            let context = v8::Local::new(
                handle_scope,
                self.contexts
                    .get(&self.active_realm)
                    .expect("active realm context exists"),
            );
            let scope = &mut v8::ContextScope::new(handle_scope, context);
            v8::tc_scope!(let scope, scope);
            let result = (|| {
                let cache_key = (current_realm_identity(scope), resource_name.into());
                let cached = cache_root
                    .then(|| {
                        scope
                            .get_slot::<ModuleCache>()
                            .and_then(|cache| cache.0.get(&cache_key))
                            .cloned()
                    })
                    .flatten();
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
                    if cache_root {
                        scope
                            .get_slot_mut::<ModuleCache>()
                            .expect("module cache installed")
                            .0
                            .insert(cache_key, global);
                    }
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
                if module.get_status() == v8::ModuleStatus::Errored {
                    let exception = module.get_exception();
                    scope.throw_exception(exception);
                    return Err(HostError::Execution("module evaluation failed".into()));
                }
                queue_mutation_observer_microtask(scope);
                let evaluation = module
                    .evaluate(scope)
                    .ok_or_else(|| HostError::Execution("module evaluation failed".into()))?;
                if report_errors {
                    let promise = v8::Local::<v8::Promise>::try_from(evaluation).map_err(|_| {
                        HostError::Execution("module evaluation did not return a promise".into())
                    })?;
                    install_module_error_reporter(scope, promise)?;
                }
                queue_mutation_observer_microtask(scope);
                scope.perform_microtask_checkpoint();
                if !report_errors
                    && let Ok(promise) = v8::Local::<v8::Promise>::try_from(evaluation)
                    && promise.state() == v8::PromiseState::Rejected
                {
                    return Err(HostError::Execution(format!(
                        "module evaluation rejected: {}",
                        promise.result(scope).to_rust_string_lossy(scope)
                    )));
                }
                Ok(())
            })();
            if deadline.expired() || scope.is_execution_terminating() {
                scope.cancel_terminate_execution();
                reset_mutation_observer_microtask_after_termination(scope);
                return Err(HostError::LimitExceeded {
                    resource: "wall time".into(),
                });
            }
            let result = if scope.has_caught() {
                let exception = scope.exception();
                let message = scope.message();
                let detail = message
                    .map(|message| message.get(scope).to_rust_string_lossy(scope))
                    .unwrap_or_else(|| "module execution failed".into());
                scope.reset();
                if report_errors && let Some(exception) = exception {
                    let resolver = v8::PromiseResolver::new(scope).ok_or_else(|| {
                        HostError::Execution("module error promise allocation failed".into())
                    })?;
                    let promise = resolver.get_promise(scope);
                    install_module_error_reporter(scope, promise)?;
                    resolver.reject(scope, exception).ok_or_else(|| {
                        HostError::Execution("module error promise rejection failed".into())
                    })?;
                    queue_mutation_observer_microtask(scope);
                    scope.perform_microtask_checkpoint();
                    Ok(())
                } else {
                    Err(HostError::Execution(detail))
                }
            } else {
                result
            };
            if deadline.expired() || scope.is_execution_terminating() {
                scope.cancel_terminate_execution();
                reset_mutation_observer_microtask_after_termination(scope);
                return Err(HostError::LimitExceeded {
                    resource: "wall time".into(),
                });
            }
            result
        }

        pub fn set_module_sources(&mut self, sources: &BTreeMap<String, String>) {
            self.isolate.set_slot(ModuleSources(sources.clone()));
        }

        pub fn register_import_map(&mut self, source: &str) -> Result<(), HostError> {
            v8::scope!(let scope, &mut self.isolate);
            let context = v8::Local::new(
                scope,
                self.contexts.get(&self.active_realm).expect("active realm"),
            );
            let scope = &mut v8::ContextScope::new(scope, context);
            current_environment(scope).map_err(HostError::Execution)?;
            let state = self.binding_states.get(&self.active_realm).ok_or_else(|| {
                HostError::Execution("import map requires a document environment".into())
            })?;
            let base = oryn_js::v8_bindings::document_base_url(scope, state, true)
                .map_err(|error| HostError::Execution(error.to_string()))?;
            let base =
                Url::parse(&base).map_err(|error| HostError::Execution(error.to_string()))?;
            module_import_map(scope)
                .borrow_mut()
                .register(source, &base)
                .map_err(HostError::Execution)
        }

        pub fn resolve_module_specifier(
            &mut self,
            base: &str,
            specifier: &str,
        ) -> Result<Option<String>, HostError> {
            let Ok(base) = Url::parse(base) else {
                return Ok(None);
            };
            v8::scope!(let scope, &mut self.isolate);
            let context = v8::Local::new(
                scope,
                self.contexts.get(&self.active_realm).expect("active realm"),
            );
            let scope = &mut v8::ContextScope::new(scope, context);
            current_environment(scope).map_err(HostError::Execution)?;
            module_import_map(scope)
                .borrow()
                .resolve(&base, specifier)
                .map_err(HostError::Execution)
        }

        pub fn take_module_error_reports(&mut self) -> Vec<String> {
            self.isolate
                .get_slot_mut::<ModuleErrorReports>()
                .map(|reports| std::mem::take(&mut reports.0))
                .unwrap_or_default()
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

    fn report_timer_exception<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        callback: v8::Local<'s, v8::Function>,
        exception: v8::Local<'s, v8::Value>,
        message: Option<v8::Local<'s, v8::Message>>,
    ) -> Result<(), HostError> {
        let context = callback
            .get_creation_context(scope)
            .unwrap_or_else(|| scope.get_current_context());
        let scope = &mut v8::ContextScope::new(scope, context);
        oryn_js::v8_bindings::report_exception(scope, exception, message)
            .map(|_| ())
            .map_err(|error| HostError::Execution(error.to_string()))
    }

    fn install_module_error_reporter(
        scope: &mut v8::PinScope,
        promise: v8::Local<v8::Promise>,
    ) -> Result<(), HostError> {
        // Allocated after snapshot restoration; this callback is never serialized.
        // V8's reaction keeps its creation realm alive until the promise settles.
        let handler = v8::Function::new(scope, module_error_report_callback)
            .ok_or_else(|| HostError::Execution("module error handler allocation failed".into()))?;
        promise.catch(scope, handler).ok_or_else(|| {
            HostError::Execution("module rejection handler installation failed".into())
        })?;
        Ok(())
    }

    fn module_error_report_callback(
        scope: &mut v8::PinScope,
        args: v8::FunctionCallbackArguments,
        _rv: v8::ReturnValue,
    ) {
        let exception = args.get(0);
        let message = v8::Exception::create_message(scope, exception);
        let detail = message.get(scope).to_rust_string_lossy(scope);
        if let Ok(false) = oryn_js::v8_bindings::report_exception(scope, exception, Some(message))
            && !scope.is_execution_terminating()
            && let Some(reports) = scope.get_slot_mut::<ModuleErrorReports>()
        {
            reports.0.push(detail);
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
                    .and_then(|value| value.to_detail_string(scope))
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
        v8::tc_scope!(let scope, scope);
        let resolver = v8::PromiseResolver::new(scope)?;
        let specifier = specifier.to_rust_string_lossy(scope);
        let resource_name = resource_name.to_rust_string_lossy(scope);
        let result = (|| -> Result<v8::Local<'s, v8::Value>, String> {
            let mut sources = scope
                .get_slot::<ModuleSources>()
                .ok_or_else(|| "module source map is unavailable".to_string())?
                .0
                .clone();
            let resolved = resolve_requested_module(scope, &resource_name, &specifier, &sources)?;
            if let Some(updated) = scope.get_slot::<ModuleSources>() {
                sources = updated.0.clone();
            }
            let cache_key = (current_realm_identity(scope), resolved.clone());
            let cached = scope
                .get_slot::<ModuleCache>()
                .and_then(|cache| cache.0.get(&cache_key))
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
                    .insert(cache_key, global);
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
                let exception = if let Some(exception) = scope.exception() {
                    scope.reset();
                    exception
                } else {
                    let detail = v8::String::new(
                        scope,
                        &format!("dynamic module {specifier:?} failed: {error}"),
                    )?;
                    v8::Exception::type_error(scope, detail)
                };
                resolver.reject(scope, exception);
            }
        }
        Some(resolver.get_promise(scope))
    }

    fn module_import_map(scope: &mut v8::PinScope) -> Rc<RefCell<crate::import_maps::ImportMap>> {
        let context = scope.get_current_context();
        if let Some(map) = context.get_slot::<RefCell<crate::import_maps::ImportMap>>() {
            return map;
        }
        context.set_slot(Rc::new(RefCell::new(
            crate::import_maps::ImportMap::default(),
        )));
        context
            .get_slot::<RefCell<crate::import_maps::ImportMap>>()
            .expect("installed import map")
    }

    fn resolve_requested_module(
        scope: &mut v8::PinScope,
        resource_name: &str,
        specifier: &str,
        sources: &BTreeMap<String, String>,
    ) -> Result<String, String> {
        current_environment(scope)?;
        let base = Url::parse(resource_name).ok().or_else(|| {
            // HTML event-handler attributes have no referencing Script Record.
            // Resolve against this realm's native document base, not the
            // isolate-wide fetch configuration or a page-overridden property.
            let state = scope.get_current_context().get_slot::<V8BindingState>()?;
            let base = oryn_js::v8_bindings::document_base_url(scope, &state, true).ok()?;
            Url::parse(&base).ok()
        });
        let map = module_import_map(scope);
        let mapped = base
            .as_ref()
            .map(|base| map.borrow().resolve(base, specifier))
            .transpose()?
            .flatten();
        let resolved = if let Some(mapped) = mapped {
            sources
                .contains_key(&mapped)
                .then_some(mapped.clone())
                .or_else(|| fetch_module_source(scope, resource_name, &mapped))
        } else {
            // Kept for explicit in-process dependency maps with non-URL names.
            // Import-map null/invalid entries have already failed above.
            resolve_module_name(resource_name, specifier, sources)
                .or_else(|| fetch_module_source(scope, resource_name, specifier))
        }
        .ok_or_else(|| {
            format!("module source {specifier:?} from {resource_name:?} is unavailable")
        })?;
        if let Some(base) = base {
            map.borrow_mut().record_resolution(&base, specifier)?;
        }
        Ok(resolved)
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
          const WorkerEvent = globalThis.Event || class WorkerEvent {
            constructor(type, init = {}) { this.type = String(type); this.defaultPrevented = false; Object.assign(this, init); }
            preventDefault() { this.defaultPrevented = true; }
          };
          class WorkerMessageEvent extends WorkerEvent {
            constructor(type, init = {}) {
              super(type, init);
              for (const [name, value] of Object.entries({data:null, origin:'', lastEventId:'', source:null, ports:[], ...init})) {
                if (!(name in this)) Object.defineProperty(this, name, {value, enumerable:true, configurable:true});
              }
            }
          }
          const WorkerEventTarget = globalThis.EventTarget || class WorkerEventTarget {
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
          };
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
          globalThis.MessageEvent = WorkerMessageEvent;
          globalThis.EventTarget = WorkerEventTarget;
          globalThis.queueMicrotask = callback => Promise.resolve().then(callback);
          globalThis.setTimeout = (callback, delay = 0, ...args) => __oryn_timer_schedule(callback, delay, false, args);
          globalThis.setInterval = (callback, delay = 0, ...args) => __oryn_timer_schedule(callback, delay, true, args);
          globalThis.clearTimeout = id => __oryn_timer_cancel(Number(id));
          globalThis.clearInterval = globalThis.clearTimeout;
          globalThis.importScripts = (...urls) => __oryn_import_scripts(...urls);
          globalThis.DedicatedWorkerGlobalScope = DedicatedWorkerGlobalScope;
          globalThis.SharedWorkerGlobalScope = SharedWorkerGlobalScope;
          globalThis.ServiceWorkerGlobalScope = ServiceWorkerGlobalScope;
          const Scope = globalThis[globalThis.__oryn_worker_global_name] || DedicatedWorkerGlobalScope;
          Object.setPrototypeOf(globalThis, Scope.prototype);
          Object.defineProperty(globalThis, Symbol.toStringTag, {value:globalThis.__oryn_worker_global_name});
          globalThis.onmessage = null;
          globalThis.onerror = null;
          globalThis.onconnect = null;
          globalThis.oninstall = null;
          class NestedWorker {
            constructor(url, options = {}) {
              this.onmessage = null;
              this.onerror = null;
              this.listeners = new Map();
              this.closed = false;
              const target = String(url);
              let response;
              try {
                response = JSON.parse(__oryn_context_call(JSON.stringify({
                  operation: 'load_worker_script',
                  page_id: __oryn_worker_page_id,
                  url: target,
                  module: String(options.type || 'classic') === 'module'
                })));
              } catch (error) {
                throw new Error(`nested worker load failed for ${target}: ${error}`);
              }
              if (response.result !== 'worker_script') throw new Error('Worker script load failed');
              try {
                this.id = __oryn_worker_create(response.source, response.url, !!response.module, 'dedicatedworker', response.referrer_policy || '');
              } catch (error) {
                throw new Error(`nested worker execution failed for ${response.url}: ${error}`);
              }
            }
            addEventListener(type, callback) {
              const key = String(type), list = this.listeners.get(key) || [];
              list.push(callback); this.listeners.set(key, list);
              if (key === 'message') queueMicrotask(() => this.poll());
            }
            removeEventListener(type, callback) {
              const key = String(type);
              this.listeners.set(key, (this.listeners.get(key) || []).filter(item => item !== callback));
            }
            dispatch(type, extra = {}) {
              const event = new WorkerEvent(type, extra);
              Object.assign(event, extra);
              for (const callback of this.listeners.get(type) || []) callback.call(this, event);
              const handler = this['on' + type];
              if (typeof handler === 'function') handler.call(this, event);
            }
            poll() {
              if (this.closed || !this.id) return;
              for (const encoded of JSON.parse(__oryn_worker_take(this.id))) {
                this.dispatch('message', {data: JSON.parse(encoded), source: null});
              }
            }
            postMessage(data) {
              if (this.closed || !this.id) return;
              const encoded = JSON.stringify(data);
              if (encoded === undefined) throw new TypeError('worker message is not structured-cloneable');
              __oryn_worker_post(this.id, encoded);
              queueMicrotask(() => this.poll());
            }
            terminate() {
              if (this.id) __oryn_worker_terminate(this.id);
              this.closed = true; this.id = 0;
            }
          }
          globalThis.Worker = NestedWorker;
          // Generated bindings already install the global EventTarget methods.
          // Keep their identity and native listener registry across worker startup.
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
          globalThis.__oryn_connect_shared = () => dispatchEvent(new MessageEvent('connect', {ports:[sharedPort],source:sharedPort}));
          globalThis.__oryn_install_service = () => dispatchEvent(new Event('install'));
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

    fn automation_trace_callback(
        scope: &mut v8::PinScope,
        args: v8::FunctionCallbackArguments,
        _: v8::ReturnValue,
    ) {
        let realm = scope.get_current_context().get_slot::<RealmDescriptor>();
        let id = args
            .get(0)
            .number_value(scope)
            .filter(|n| n.is_finite() && *n > 0.0 && *n <= 9007199254740991.0 && n.fract() == 0.0);
        let operation = args
            .get(1)
            .is_string()
            .then(|| args.get(1).to_rust_string_lossy(scope));
        let outcome = args
            .get(2)
            .is_string()
            .then(|| args.get(2).to_rust_string_lossy(scope));
        let Some(trace) = scope.get_slot_mut::<crate::automation_trace::AutomationTrace>() else {
            throw_type_error(scope, "automation trace unavailable");
            return;
        };
        let accepted = if let (Some(realm), Some(id), Some(operation), Some(outcome)) =
            (realm, id, operation, outcome)
        {
            trace.record(
                realm.id.0,
                realm.document_generation,
                id as u64,
                &operation,
                &outcome,
            )
        } else {
            trace.invalidate();
            false
        };
        if !accepted {
            throw_type_error(
                scope,
                "automation trace rejected invalid or excessive observation",
            );
        }
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
        #[cfg(unix)]
        let platform_context = scope.get_slot::<ContextState>().cloned();
        let created = (|| -> Result<u64, String> {
            let owner_environment = current_environment(scope)?;
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
            let referrer_policy = args
                .get(4)
                .to_string(scope)
                .map(|value| value.to_rust_string_lossy(scope))
                .unwrap_or_default();
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
                registry.next_id = worker_id
                    .checked_add(1)
                    .ok_or("worker identity space exhausted")?;
                worker_id
            };
            let worker_realm = u32::try_from(worker_id)
                .ok()
                .and_then(|id| id.checked_add(10_000))
                .ok_or("worker realm identity space exhausted")?;
            let environment = scope
                .get_slot_mut::<AgentEnvironments>()
                .ok_or("agent environments unavailable")?
                .create(RealmId(worker_realm), 1, Some(owner_environment.id))?;
            let context = v8::Context::new(scope, Default::default());
            context.set_slot(Rc::new(environment));
            install_host_functions!(scope, context);
            let context_global = v8::Global::new(scope, context);
            scope
                .get_slot_mut::<DedicatedWorkers>()
                .expect("dedicated-worker registry installed")
                .workers
                .insert(
                    worker_id,
                    DedicatedWorker {
                        environment,
                        owner_identity,
                        context: context_global,
                        binding_state: None,
                        closed: false,
                    },
                );

            let executed = (|| -> Result<(), String> {
                let worker_scope = &mut v8::ContextScope::new(scope, context);
                let runtime = oryn_dom::html::parse("<!doctype html>")
                    .into_runtime()
                    .map_err(|error| error.to_string())?;
                let document = runtime.document();
                let dom = Rc::new(RefCell::new(runtime));
                let binding_state = install_generated_bindings_with_root_for_global(
                    worker_scope,
                    context,
                    dom,
                    RealmId(worker_realm),
                    1,
                    document,
                    BindingGlobal::Worker,
                )
                .map_err(|error| error.to_string())?;
                #[cfg(unix)]
                if let Some(platform_context) = platform_context.clone() {
                    binding_state
                        .set_platform_host(Rc::new(ContextPlatformHost::new(platform_context)));
                }
                worker_scope
                    .get_slot_mut::<DedicatedWorkers>()
                    .expect("dedicated-worker registry installed")
                    .workers
                    .get_mut(&worker_id)
                    .expect("dedicated worker registered before initialization")
                    .binding_state = Some(binding_state.clone());
                let id_name = v8::String::new(worker_scope, "__oryn_worker_id")
                    .ok_or_else(|| "worker id allocation failed".to_string())?;
                worker_scope.get_current_context().global(worker_scope).set(
                    worker_scope,
                    id_name.into(),
                    v8::Number::new(worker_scope, worker_id as f64).into(),
                );
                #[cfg(unix)]
                if let Some(platform_context) = platform_context.as_ref() {
                    let page_name = v8::String::new(worker_scope, "__oryn_worker_page_id")
                        .ok_or_else(|| "worker page id allocation failed".to_string())?;
                    worker_scope.get_current_context().global(worker_scope).set(
                        worker_scope,
                        page_name.into(),
                        v8::Number::new(worker_scope, platform_context.page_id.0 as f64).into(),
                    );
                }
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
                binding_state.set_security_origin(
                    parsed_url
                        .as_ref()
                        .map(|url| url.origin().ascii_serialization())
                        .unwrap_or_else(|| "null".into()),
                );
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
                    "globalThis.location=Object.freeze({{href:{url},origin:{origin},pathname:{pathname},toString(){{return this.href}}}});globalThis.__oryn_worker_global_name={global_name};globalThis.__oryn_referrer_policy={};{}{}",
                    serde_json::to_string(&referrer_policy).map_err(|error| error.to_string())?,
                    DEDICATED_WORKER_BOOTSTRAP,
                    crate::observable::OBSERVABLE_GLOBALS,
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
                    let evaluation = module
                        .evaluate(worker_scope)
                        .ok_or_else(|| "worker module evaluation failed".to_string())?;
                    queue_mutation_observer_microtask(worker_scope);
                    worker_scope.perform_microtask_checkpoint();
                    if let Ok(promise) = v8::Local::<v8::Promise>::try_from(evaluation)
                        && promise.state() == v8::PromiseState::Rejected
                    {
                        return Err(format!(
                            "worker module evaluation rejected: {}",
                            promise
                                .result(worker_scope)
                                .to_rust_string_lossy(worker_scope)
                        ));
                    }
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
                } else if global_kind == WorkerGlobalKind::Service {
                    let install = v8::String::new(worker_scope, "__oryn_install_service()")
                        .ok_or_else(|| "service-worker install allocation failed".to_string())?;
                    let install = v8::Script::compile(worker_scope, install, None)
                        .ok_or_else(|| "service-worker install compilation failed".to_string())?;
                    install
                        .run(worker_scope)
                        .ok_or_else(|| "service-worker install dispatch failed".to_string())?;
                }
                queue_mutation_observer_microtask(worker_scope);
                worker_scope.perform_microtask_checkpoint();
                Ok(())
            })();
            if let Err(error) = executed {
                retire_environment(scope, environment.id);
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

    fn import_scripts_callback(
        scope: &mut v8::PinScope,
        args: v8::FunctionCallbackArguments,
        mut return_value: v8::ReturnValue<v8::Value>,
    ) {
        #[cfg(unix)]
        let context = scope.get_slot::<ContextState>().cloned();
        let result = (|| -> Result<(), String> {
            let location = scope
                .get_current_context()
                .global(scope)
                .get(
                    scope,
                    v8::String::new(scope, "location")
                        .ok_or_else(|| "worker location allocation failed".to_string())?
                        .into(),
                )
                .and_then(|value| value.to_object(scope))
                .and_then(|location| {
                    location.get(
                        scope,
                        v8::String::new(scope, "href")
                            .expect("static worker href name")
                            .into(),
                    )
                })
                .and_then(|value| value.to_string(scope))
                .map(|value| value.to_rust_string_lossy(scope))
                .ok_or_else(|| "worker location is unavailable".to_string())?;
            for index in 0..args.length() {
                let requested = args
                    .get(index)
                    .to_string(scope)
                    .map(|value| value.to_rust_string_lossy(scope))
                    .ok_or_else(|| "importScripts URL is not string-convertible".to_string())?;
                #[cfg(unix)]
                let (url, source) = {
                    let context = context.as_ref().ok_or_else(|| {
                        "importScripts context service is unavailable".to_string()
                    })?;
                    match context
                        .client
                        .request(ContextServiceRequest::LoadScript {
                            page_id: context.page_id,
                            url: requested,
                            module: false,
                            nonce: None,
                            initiator_url: Some(location.clone()),
                            referrer_policy: None,
                        })
                        .map_err(|error| error.to_string())?
                    {
                        ContextServiceResponse::ScriptSource { url, source, .. } => (url, source),
                        _ => return Err("importScripts received an invalid response".into()),
                    }
                };
                #[cfg(not(unix))]
                let (url, source): (String, String) = {
                    let _ = (&requested, &location);
                    return Err("importScripts requires the parent context service".into());
                };
                let source = v8::String::new(scope, &source)
                    .ok_or_else(|| "importScripts source allocation failed".to_string())?;
                let resource = v8::String::new(scope, &url)
                    .ok_or_else(|| "importScripts resource-name allocation failed".to_string())?;
                let origin = v8::ScriptOrigin::new(
                    scope,
                    resource.into(),
                    0,
                    0,
                    false,
                    0,
                    None,
                    false,
                    false,
                    false,
                    None,
                );
                let script = v8::Script::compile(scope, source, Some(&origin))
                    .ok_or_else(|| format!("importScripts compilation failed for {url}"))?;
                script
                    .run(scope)
                    .ok_or_else(|| format!("importScripts execution failed for {url}"))?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => return_value.set(v8::undefined(scope).into()),
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
            let try_catch = std::pin::pin!(v8::TryCatch::new(worker_scope));
            let mut try_catch = try_catch.init();
            let source =
                v8::String::new(&try_catch, &format!("__oryn_dispatch_message({encoded})"))
                    .ok_or_else(|| "worker message allocation failed".to_string())?;
            let script = v8::Script::compile(&try_catch, source, None)
                .ok_or_else(|| "worker message compilation failed".to_string())?;
            if script.run(&try_catch).is_none() {
                let detail = try_catch
                    .exception()
                    .map(|value| value.to_rust_string_lossy(&try_catch))
                    .unwrap_or_else(|| "unknown exception".into());
                return Err(format!("worker message dispatch failed: {detail}"));
            }
            queue_mutation_observer_microtask(&mut try_catch);
            try_catch.perform_microtask_checkpoint();
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
            let environment = registry
                .workers
                .remove(&worker_id)
                .map(|worker| worker.environment);
            registry.outbound.remove(&worker_id);
            if let Some(environment) = environment {
                retire_environment(scope, environment.id);
            }
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
            let environment = worker.environment;
            retire_environment(scope, environment.id);
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
            current_environment(scope)?;
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
        let image_request = matches!(request, PlatformHostRequest::ImageStart { .. });
        match request {
            PlatformHostRequest::HttpStart { request }
            | PlatformHostRequest::ImageStart { request } => {
                let credentials = match request.credentials.as_str() {
                    "include" => true,
                    "omit" => false,
                    _ => request
                        .initiator_url
                        .as_deref()
                        .and_then(|source| url::Url::parse(source).ok())
                        .zip(url::Url::parse(&request.url).ok())
                        .is_some_and(|(source, target)| source.origin() == target.origin()),
                };
                let stream_request = HttpStreamRequest {
                    request_id: operation_id.0,
                    method: request.method,
                    url: request.url.clone(),
                    headers: request.headers,
                    body_base64: BASE64.encode(request.body),
                    credentials,
                    credentials_mode: Some(request.credentials),
                    initiator_url: request.initiator_url,
                    referrer_policy: request.referrer_policy,
                };
                let command = if image_request {
                    ContextServiceRequest::ImageStreamStart {
                        page_id: state.page_id,
                        request: stream_request,
                    }
                } else {
                    ContextServiceRequest::HttpStreamStart {
                        page_id: state.page_id,
                        request: stream_request,
                    }
                };
                match state
                    .client
                    .request(command)
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
                    credentials_mode: None,
                    initiator_url: None,
                    referrer_policy: None,
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
            let environment = current_environment(scope)?;
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
            operations.next_operation_id = operation_id
                .checked_add(1)
                .ok_or("asynchronous operation identity space exhausted")?;
            if operations.pending.len() + operations.retired.len() >= 32 {
                return Err("pending host operation limit exceeded".into());
            }
            operations.pending.insert(
                operation_id,
                PendingFetch {
                    environment,
                    context,
                    resolver,
                },
            );
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
            let environment = current_environment(scope)?;
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
            async_fetches.next_operation_id = operation_id
                .checked_add(1)
                .ok_or("asynchronous operation identity space exhausted")?;
            if async_fetches.pending.len() + async_fetches.retired.len() >= 32 {
                return Err("pending fetch limit exceeded".into());
            }
            async_fetches.pending.insert(
                operation_id,
                PendingFetch {
                    environment,
                    context,
                    resolver,
                },
            );
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
            if url.scheme() != "data" && origin(&url) != request_origin {
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
        let resolved = match resolve_requested_module(scope, &resource_name, &specifier, &sources) {
            Ok(resolved) => resolved,
            Err(error) => {
                if let Some(failures) = scope.get_slot_mut::<ModuleResolutionFailures>() {
                    failures.0.push(error.clone());
                }
                if let Some(message) = v8::String::new(scope, &error) {
                    let exception = v8::Exception::type_error(scope, message);
                    scope.throw_exception(exception);
                }
                return None;
            }
        };
        let cache_key = (current_realm_identity(scope), resolved.clone());
        if let Some(module) = scope
            .get_slot::<ModuleCache>()
            .and_then(|cache| cache.0.get(&cache_key))
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
            .insert(cache_key, global);
        Some(module)
    }

    fn fetch_module_source(
        scope: &mut v8::PinScope<'_, '_>,
        resource_name: &str,
        specifier: &str,
    ) -> Option<String> {
        current_environment(scope).ok()?;
        if !(specifier.starts_with('.')
            || specifier.starts_with('/')
            || Url::parse(specifier).is_ok())
        {
            return None;
        }
        let fetch = scope.get_slot::<FetchState>().cloned();
        let fallback_base = fetch.as_ref().map(|state| state.base_url.clone());
        let base = if resource_name.trim().is_empty() || resource_name == "undefined" {
            fallback_base
        } else {
            Url::parse(resource_name)
                .ok()
                .or_else(|| {
                    fallback_base
                        .as_ref()
                        .and_then(|base| base.join(resource_name).ok())
                })
                .or(fallback_base)
        }?;
        let url = base.join(specifier).ok()?;
        #[cfg(unix)]
        if let Some(context) = scope.get_slot::<ContextState>().cloned() {
            let referrer_policy = scope
                .get_current_context()
                .global(scope)
                .get(
                    scope,
                    v8::String::new(scope, "__oryn_referrer_policy")?.into(),
                )
                .filter(|value| !value.is_null_or_undefined())
                .and_then(|value| value.to_string(scope))
                .map(|value| value.to_rust_string_lossy(scope))
                .filter(|value| !value.trim().is_empty());
            let response = context
                .client
                .request(ContextServiceRequest::LoadScript {
                    page_id: context.page_id,
                    url: url.to_string(),
                    module: true,
                    nonce: None,
                    initiator_url: Some(base.to_string()),
                    referrer_policy,
                })
                .ok()?;
            let ContextServiceResponse::ScriptSource { url, source, .. } = response else {
                return None;
            };
            scope
                .get_slot_mut::<ModuleSources>()?
                .0
                .insert(url.clone(), source);
            return Some(url);
        }
        let fetch = fetch?;
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
            self.watchdog.shutdown();
            // `OwnedIsolate` requires itself to be current during destruction.
            // A BFCache-retained host is deliberately exited while frozen, so
            // re-enter it for the automatic field drop and restore the prior
            // isolate when `OwnedIsolate` exits.
            if !self.isolate_entered {
                // SAFETY: the page actor drops hosts on their creating thread.
                unsafe { self.isolate.enter() };
                self.isolate_entered = true;
            }
            for state in self.binding_states.values() {
                state.retire_execution();
            }
            if let Some(workers) = self.isolate.get_slot::<DedicatedWorkers>() {
                for worker in workers.workers.values() {
                    if let Some(state) = &worker.binding_state {
                        state.retire_execution();
                    }
                }
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
            queue_mutation_observer_microtask(scope);
            if script.run(scope).is_none() {
                if _deadline.expired() || scope.is_execution_terminating() {
                    scope.cancel_terminate_execution();
                    reset_mutation_observer_microtask_after_termination(scope);
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
            queue_mutation_observer_microtask(scope);
            scope.perform_microtask_checkpoint();
            if _deadline.expired() || scope.is_execution_terminating() {
                scope.cancel_terminate_execution();
                reset_mutation_observer_microtask_after_termination(scope);
                return Err(HostError::LimitExceeded {
                    resource: "wall time".into(),
                });
            }
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
            Ok(())
        }

        fn evaluate_string(&mut self, source: &str) -> Result<String, HostError> {
            // Host-only diagnostic command: no page-visible reader and no
            // traversal through cross-origin script proxies.
            if source == "__oryn_native_automation_trace" {
                return self
                    .isolate
                    .get_slot::<crate::automation_trace::AutomationTrace>()
                    .map(|trace| trace.snapshot().to_string())
                    .ok_or_else(|| HostError::Execution("automation trace unavailable".into()));
            }
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
                    reset_mutation_observer_microtask_after_termination(scope);
                    return Err(HostError::LimitExceeded {
                        resource: "wall time".into(),
                    });
                }
                None => {
                    let detail = scope
                        .stack_trace()
                        .filter(|value| !value.is_null_or_undefined())
                        .or_else(|| scope.exception())
                        .and_then(|value| value.to_string(scope))
                        .map(|value| value.to_rust_string_lossy(scope))
                        .unwrap_or_else(|| "execution failed".into());
                    return Err(HostError::Execution(detail));
                }
            };
            if _deadline.expired() || scope.is_execution_terminating() {
                scope.cancel_terminate_execution();
                reset_mutation_observer_microtask_after_termination(scope);
                return Err(HostError::LimitExceeded {
                    resource: "wall time".into(),
                });
            }
            let value = value.to_string(scope);
            if _deadline.expired() || scope.is_execution_terminating() {
                scope.cancel_terminate_execution();
                reset_mutation_observer_microtask_after_termination(scope);
                return Err(HostError::LimitExceeded {
                    resource: "wall time".into(),
                });
            }
            let value = value
                .ok_or_else(|| HostError::Execution("result is not string-convertible".into()))?;
            Ok(value.to_rust_string_lossy(scope))
        }

        fn terminate(&mut self) {
            self.isolate.terminate_execution();
        }
    }
    #[cfg(test)]
    mod native_context_tests {
        #[test]
        fn timer_nesting_clamps_callbacks_but_not_their_microtasks() {
            use super::*;
            assert_eq!(nested_timer_delay(0, 5), Duration::ZERO);
            assert_eq!(nested_timer_delay(0, 6), Duration::from_millis(4));
            assert_eq!(nested_timer_delay(9, 6), Duration::from_millis(9));
            let mut host = RawV8Host::new();
            host.execute("globalThis.root=__oryn_timer_schedule(()=>{globalThis.nested=__oryn_timer_schedule(()=>{},10000,false,[]);Promise.resolve().then(()=>{globalThis.micro=__oryn_timer_schedule(()=>{},10000,false,[])})},0,false,[])").unwrap();
            let root: u64 = host
                .evaluate_string("String(root)")
                .unwrap()
                .parse()
                .unwrap();
            host.isolate
                .get_slot_mut::<TimerRegistry>()
                .unwrap()
                .callbacks
                .get_mut(&root)
                .unwrap()
                .nesting_level = 6;
            host.drain_scheduled_timers(false).unwrap();
            let nested: u64 = host
                .evaluate_string("String(nested)")
                .unwrap()
                .parse()
                .unwrap();
            let micro: u64 = host
                .evaluate_string("String(micro)")
                .unwrap()
                .parse()
                .unwrap();
            let registry = host.isolate.get_slot::<TimerRegistry>().unwrap();
            assert_eq!(registry.callbacks[&nested].nesting_level, 7);
            assert_eq!(registry.callbacks[&micro].nesting_level, 1);
            assert_eq!(registry.task_nesting.get(), 0);
        }

        #[test]
        fn interval_reinitialization_keeps_public_handle_and_obeys_microtask_cancel() {
            use super::*;
            let mut host = RawV8Host::new();
            host.execute("globalThis.count=0;globalThis.interval=__oryn_timer_schedule(()=>{count++;if(count===2)Promise.resolve().then(()=>__oryn_timer_cancel(interval))},0,true,[])").unwrap();
            let id: u64 = host
                .evaluate_string("String(interval)")
                .unwrap()
                .parse()
                .unwrap();
            assert_eq!(host.drain_scheduled_timers(false).unwrap(), 1);
            let timer = &host.isolate.get_slot::<TimerRegistry>().unwrap().callbacks[&id];
            assert_ne!(timer.scheduled_id, id);
            assert_eq!(timer.nesting_level, 2);
            assert_eq!(host.drain_scheduled_timers(false).unwrap(), 1);
            assert_eq!(host.evaluate_string("String(count)").unwrap(), "2");
            assert_eq!(host.next_timer_delay(), None);
            assert!(
                host.isolate
                    .get_slot::<TimerRegistry>()
                    .unwrap()
                    .callbacks
                    .is_empty()
            );
        }

        #[test]
        fn interval_cancellation_removes_the_reinitialized_scheduler_handle() {
            use super::*;
            let mut host = RawV8Host::new();
            host.execute("globalThis.interval=__oryn_timer_schedule(()=>{},0,true,[])")
                .unwrap();
            host.drain_scheduled_timers(false).unwrap();
            host.execute("__oryn_timer_cancel(interval)").unwrap();
            assert_eq!(host.next_timer_delay(), None);
            assert_eq!(host.drain_scheduled_timers(false).unwrap(), 0);
        }

        use super::*;

        #[cfg(unix)]
        #[test]
        fn retired_platform_endpoint_rejects_retained_clones() {
            let context = crate::Browser::new().new_context();
            let page = context.new_page();
            let endpoint = Rc::new(ContextPlatformHost::new(ContextState {
                client: SharedContextClient::direct(context, page.page_id()),
                page_id: page.page_id(),
            }));
            let retained = endpoint.clone();
            endpoint.retire();
            drop(endpoint);
            assert_eq!(
                retained
                    .submit(
                        HostOperationId(1),
                        PlatformHostRequest::HttpRead {
                            stream_id: 1,
                            max_bytes: 16,
                        }
                    )
                    .unwrap_err(),
                "execution environment is retired"
            );
            assert!(retained.drain_completions(32).is_empty());
        }

        #[test]
        fn adopted_cross_realm_wrapper_survives_source_retirement_and_gc() {
            let mut host = host();
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body></body>")
                    .into_runtime()
                    .unwrap(),
            ));
            host.install_dom(dom.clone(), 1).unwrap();
            let parsed = oryn_dom::html::parse("<section id=kept><span>text</span></section>");
            let child_document = dom
                .borrow_mut()
                .import_subtree(&parsed.dom, parsed.document)
                .unwrap();
            let child = host
                .create_realm_with_dom_root(
                    "https://example.test",
                    2,
                    Some(1),
                    dom.clone(),
                    child_document,
                )
                .unwrap();
            expose_child(&mut host, child);
            host.execute("globalThis.kept=child.document.getElementById('kept');kept.marker=73;document.adoptNode(kept);globalThis.savedOwner=kept.ownerDocument;globalThis.child=null").unwrap();
            host.remove_realm(child).unwrap();
            let mutations = dom.borrow_mut().drain_mutations();
            dom.borrow_mut().release_mutation_roots(&mutations).unwrap();
            host.isolate.low_memory_notification();
            host.isolate.low_memory_notification();
            assert_eq!(host.evaluate_string("kept.marker===73 && kept.ownerDocument===document && savedOwner===document && kept.firstChild.textContent==='text' && kept.parentNode===null").unwrap(), "true");
            host.execute("document.body.append(kept)").unwrap();
            assert_eq!(
                host.evaluate_string("kept.parentNode===document.body")
                    .unwrap(),
                "true"
            );
        }

        #[test]
        fn retained_retired_global_cannot_schedule_new_timers() {
            let mut host = host();
            let child = host
                .create_realm("https://example.test", 2, Some(1))
                .unwrap();
            expose_child(&mut host, child);
            host.execute("globalThis.retainedSchedule=child.__oryn_timer_schedule")
                .unwrap();
            host.remove_realm(child).unwrap();
            assert_eq!(
                host.evaluate_string(
                    "retainedSchedule(()=>{throw Error('retired timer')},0,false,[])"
                )
                .unwrap(),
                "0"
            );
            assert_eq!(host.next_timer_delay(), None);
        }

        #[test]
        fn native_security_tokens_follow_structured_origin_identity() {
            let mut host = host();
            let normalized = host
                .create_realm("https://EXAMPLE.test:443/path", 1, Some(1))
                .unwrap();
            expose_child(&mut host, normalized);
            host.execute("child.shared = {value: 7}").unwrap();
            host.activate_realm(normalized).unwrap();
            assert_eq!(host.evaluate_string("String(shared.value)").unwrap(), "7");
            host.activate_realm(RealmId(0)).unwrap();
            let opaque = host.create_realm("null", 1, Some(2)).unwrap();
            expose_child(&mut host, opaque);
            assert_eq!(
                host.evaluate_string(
                    "(()=>{try {child.secret=1;return 'allowed'}catch(e){return 'denied'}})()"
                )
                .unwrap(),
                "denied"
            );
            let origin = Origin::opaque();
            let first = host.create_realm(origin.clone(), 1, Some(3)).unwrap();
            let inherited = host.create_realm(origin, 1, Some(4)).unwrap();
            assert!(
                host.isolate
                    .get_slot::<NativeRealmIdentities>()
                    .unwrap()
                    .realms
                    .same_origin(first, inherited)
            );
            v8::scope!(let scope, &mut host.isolate);
            let first = v8::Local::new(scope, host.contexts.get(&first).unwrap());
            let inherited = v8::Local::new(scope, host.contexts.get(&inherited).unwrap());
            assert!(
                first
                    .get_security_token(scope)
                    .strict_equals(inherited.get_security_token(scope))
            );
        }

        fn host() -> RawV8Host {
            RawV8Host::new_with_base_url_and_heap_limit(
                Url::parse("https://example.test/").unwrap(),
                256 * 1024 * 1024,
            )
        }

        #[test]
        fn retired_creator_cannot_allocate_child_document() {
            let mut host = host();
            let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
            host.install_dom(dom.clone(), 1).unwrap();
            {
                let parent = host.contexts.get(&RealmId(0)).unwrap().clone();
                v8::scope!(let scope, &mut host.isolate);
                let parent = v8::Local::new(scope, &parent);
                let environment = *parent.get_slot::<EnvironmentDescriptor>().unwrap();
                retire_environment(scope, environment.id);
            }
            assert!(
                host.create_realm_with_dom("https://example.test", 1, Some(1), dom)
                    .is_err()
            );
            assert_eq!(host.active_realm(), RealmId(0));
            assert_eq!(host.contexts.len(), 1);
            assert!(host.realm_descriptor(RealmId(1)).is_none());
            assert!(host.adopt_created_realms().is_empty());
        }

        #[test]
        fn failed_child_document_installation_is_not_adoptable() {
            let mut host = host();
            let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
            host.install_dom(dom.clone(), 1).unwrap();
            let invalid = NodeId {
                slot: u32::MAX,
                generation: u32::MAX,
            };
            assert!(
                host.create_realm_with_dom_root(
                    "https://example.test",
                    1,
                    Some(1),
                    dom.clone(),
                    invalid
                )
                .is_err()
            );
            assert_eq!(host.active_realm(), RealmId(0));
            assert_eq!(host.contexts.len(), 1);
            assert_eq!(host.realm_ids(), vec![RealmId(0)]);
            assert!(host.adopt_created_realms().is_empty());
            assert!(host.realm_descriptor(RealmId(1)).is_none());
            let document = dom.borrow().document();
            let next = host
                .create_realm_with_dom_root("https://example.test", 1, Some(2), dom, document)
                .unwrap();
            assert_eq!(next, RealmId(2));
            assert_eq!(host.active_realm(), RealmId(0));
            assert!(host.realm_execution_live(next));
        }

        #[test]
        fn scope_frame_lifetime_rejects_wrong_owner_and_retires_on_removal() {
            fn bind_from_callback(
                scope: &mut v8::PinScope,
                args: v8::FunctionCallbackArguments,
                mut rv: v8::ReturnValue,
            ) {
                let parent = scope.get_current_context();
                let target = v8::Local::<v8::Object>::try_from(args.data()).unwrap();
                let child = target.get_creation_context(scope).unwrap();
                let state = parent.get_slot::<V8BindingState>().unwrap();
                let frame = state.dom().borrow().find_by_id("frame").unwrap();
                bind_native_frame_lifetime(scope, parent, child, frame).unwrap();
                assert_eq!(scope.get_current_context(), parent);
                rv.set_bool(true);
            }
            let mut host = host();
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<iframe id=frame></iframe><div id=notframe></div>")
                    .into_runtime()
                    .unwrap(),
            ));
            host.install_dom(dom.clone(), 1).unwrap();
            let parent = host.contexts.get(&RealmId(0)).unwrap().clone();
            let frame = dom.borrow().find_by_id("frame").unwrap();
            let notframe = dom.borrow().find_by_id("notframe").unwrap();
            v8::scope!(let scope, &mut host.isolate);
            let parent = v8::Local::new(scope, &parent);
            let (realm, child) = create_native_realm_context(
                scope,
                parent.get_slot::<RealmDescriptor>().unwrap().origin.clone(),
                1,
                Some(1),
                None,
            )
            .unwrap();
            let child = v8::Local::new(scope, &child);
            let child_dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
            let document = child_dom.borrow().document();
            let child_state =
                install_native_document(scope, child, child_dom, realm, 1, document, None).unwrap();
            assert!(bind_native_frame_lifetime(scope, parent, child, notframe).is_err());
            assert!(scope.get_slot::<FrameLifetimes>().unwrap().live.is_empty());
            {
                let scope = &mut v8::ContextScope::new(scope, parent);
                let callback = v8::Function::builder(bind_from_callback)
                    .data(child.global(scope).into())
                    .build(scope)
                    .unwrap();
                let key = v8::String::new(scope, "bindNativeFrame").unwrap();
                parent.global(scope).set(scope, key.into(), callback.into());
                let source = v8::String::new(scope, "bindNativeFrame()").unwrap();
                assert!(
                    v8::Script::compile(scope, source, None)
                        .unwrap()
                        .run(scope)
                        .unwrap()
                        .is_true()
                );
            }
            let lifetime = child.get_slot::<FrameLifetime>().unwrap();
            assert!(!lifetime.closed.get());
            {
                let scope = &mut v8::ContextScope::new(scope, parent);
                let source =
                    v8::String::new(scope, "document.getElementById('frame').remove()").unwrap();
                v8::Script::compile(scope, source, None)
                    .unwrap()
                    .run(scope)
                    .unwrap();
            }
            {
                let scope = &mut v8::ContextScope::new(scope, parent);
                checkpoint_frame_lifetimes(scope);
            }
            assert!(lifetime.closed.get());
            assert!(child_state.execution_retired());
            assert!(bind_native_frame_lifetime(scope, parent, child, frame).is_err());
            assert!(scope.get_slot::<FrameLifetimes>().unwrap().live.is_empty());
        }

        fn expose_child(host: &mut RawV8Host, child: RealmId) {
            let parent = host.contexts.get(&RealmId(0)).unwrap().clone();
            let child = host.contexts.get(&child).unwrap().clone();
            v8::scope!(let scope, &mut host.isolate);
            let parent = v8::Local::new(scope, &parent);
            let scope = &mut v8::ContextScope::new(scope, parent);
            let child = v8::Local::new(scope, &child);
            let global = child.global(scope);
            let key = v8::String::new(scope, "child").unwrap();
            assert_eq!(
                parent.global(scope).set(scope, key.into(), global.into()),
                Some(true)
            );
        }

        fn inspect_incumbent(
            scope: &mut v8::PinScope,
            _: v8::FunctionCallbackArguments,
            mut rv: v8::ReturnValue,
        ) {
            let current = scope
                .get_current_context()
                .get_slot::<RealmDescriptor>()
                .unwrap();
            let incumbent = RawV8Host::incumbent_realm(scope).unwrap();
            let result = format!("{}:{}", current.id.0, incumbent.id.0);
            rv.set(v8::String::new(scope, &result).unwrap().into());
        }

        #[test]
        fn child_blank_document_captures_creator_base_url() {
            let mut host = host();
            let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
            host.install_dom(dom, 1).unwrap();
            host.commit_location("https://example.test/parent/page")
                .unwrap();
            let child = host
                .create_realm_with_dom(
                    "https://example.test",
                    1,
                    Some(1),
                    Rc::new(RefCell::new(oryn_dom::DomRuntime::new())),
                )
                .unwrap();
            host.commit_location("https://example.test/later/page")
                .unwrap();
            host.activate_realm(child).unwrap();
            assert_eq!(host.evaluate_string("document.URL").unwrap(), "about:blank");
            assert_eq!(
                host.evaluate_string("document.baseURI").unwrap(),
                "https://example.test/parent/page"
            );
            host.commit_location("https://other.test/committed/page")
                .unwrap();
            assert_eq!(
                host.evaluate_string("document.baseURI").unwrap(),
                "https://other.test/committed/page"
            );
        }

        #[test]
        fn nested_blank_document_captures_creator_base_element() {
            let mut host = host();
            let dom = oryn_dom::html::parse("<!doctype html><base href='../assets/'>")
                .into_runtime()
                .unwrap();
            host.install_dom(Rc::new(RefCell::new(dom)), 1).unwrap();
            host.commit_location("https://example.test/parent/page")
                .unwrap();
            let child = host
                .create_realm_with_dom(
                    "https://example.test",
                    1,
                    Some(1),
                    Rc::new(RefCell::new(oryn_dom::DomRuntime::new())),
                )
                .unwrap();
            host.execute("document.querySelector('base').href = '/changed/'")
                .unwrap();
            host.activate_realm(child).unwrap();
            assert_eq!(
                host.evaluate_string("document.baseURI").unwrap(),
                "https://example.test/assets/"
            );
            let grandchild = host
                .create_realm_with_dom(
                    "https://example.test",
                    1,
                    Some(2),
                    Rc::new(RefCell::new(oryn_dom::DomRuntime::new())),
                )
                .unwrap();
            host.commit_location("https://example.test/child-navigation")
                .unwrap();
            host.activate_realm(grandchild).unwrap();
            assert_eq!(host.evaluate_string("document.URL").unwrap(), "about:blank");
            assert_eq!(
                host.evaluate_string("document.baseURI").unwrap(),
                "https://example.test/assets/"
            );
        }

        #[test]
        fn native_document_installer_rejects_invalid_environment_before_mutation() {
            let mut host = host();
            let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
            let document = dom.borrow().document();
            host.install_dom(dom.clone(), 1).unwrap();
            let context = host.contexts.get(&RealmId(0)).unwrap().clone();
            v8::scope!(let scope, &mut host.isolate);
            let context = v8::Local::new(scope, &context);
            let previous = context.get_slot::<V8BindingState>().unwrap();
            let result = install_native_document(
                scope,
                context,
                dom.clone(),
                RealmId(99),
                1,
                document,
                None,
            );
            assert!(
                matches!(result, Err(HostError::Execution(detail)) if detail == "document realm identity mismatch")
            );
            assert!(Rc::ptr_eq(
                &previous,
                &context.get_slot::<V8BindingState>().unwrap()
            ));
            assert!(!previous.execution_retired());
            let environment = *context.get_slot::<EnvironmentDescriptor>().unwrap();
            retire_environment(scope, environment.id);
            let result =
                install_native_document(scope, context, dom, RealmId(0), 2, document, None);
            assert!(
                matches!(result, Err(HostError::Execution(detail)) if detail == "execution environment is retired")
            );
            assert!(Rc::ptr_eq(
                &previous,
                &context.get_slot::<V8BindingState>().unwrap()
            ));
        }

        #[test]
        fn native_document_policy_belongs_to_its_realm_not_last_configured_sibling() {
            for parent_restricted in [true, false] {
                let mut host = host();
                let dom = || Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
                let policy = |restricted| oryn_js::trusted_types::TrustedTypesConfig {
                    require_script: restricted,
                    deny_policy_creation: restricted,
                    ..Default::default()
                };
                let check = "(()=>{let html=false,code=false;try{document.createElement('div').innerHTML='<b>raw</b>'}catch(_){html=true}try{eval('1')}catch(_){code=true}return [html,code].join(',')})()";
                host.install_dom(dom(), 1).unwrap();
                host.configure_trusted_types(policy(parent_restricted), !parent_restricted)
                    .unwrap();
                let child = host
                    .create_realm_with_dom("https://example.test", 1, Some(1), dom())
                    .unwrap();
                host.activate_realm(child).unwrap();
                host.configure_trusted_types(policy(!parent_restricted), parent_restricted)
                    .unwrap();
                host.activate_realm(RealmId(0)).unwrap();
                let expected = format!("{parent_restricted},{parent_restricted}");
                assert_eq!(
                    host.evaluate_string(check).unwrap(),
                    expected,
                    "configuring a child must preserve the parent's policy"
                );
                let sibling = host
                    .create_realm_with_dom("https://example.test", 1, Some(2), dom())
                    .unwrap();
                host.activate_realm(sibling).unwrap();
                assert_eq!(
                    host.evaluate_string(check).unwrap(),
                    expected,
                    "legacy realm inheritance must select the active creator, not the last configured sibling"
                );
                host.activate_realm(child).unwrap();
                assert_eq!(
                    host.evaluate_string(check).unwrap(),
                    format!("{},{}", !parent_restricted, !parent_restricted)
                );
            }
        }

        #[test]
        fn native_realm_factory_runs_inside_callback_with_document_and_origin_identity() {
            fn create_child(
                scope: &mut v8::PinScope,
                args: v8::FunctionCallbackArguments,
                mut rv: v8::ReturnValue,
            ) {
                let caller = scope.get_current_context();
                let origin = if args.get(0).boolean_value(scope) {
                    Origin::opaque()
                } else {
                    caller.get_slot::<RealmDescriptor>().unwrap().origin.clone()
                };
                let dom = Rc::new(RefCell::new(oryn_dom::DomRuntime::new()));
                let document = dom.borrow().document();
                let (_, context) = create_native_document_context(
                    scope,
                    caller,
                    origin,
                    1,
                    Some(7),
                    dom,
                    document,
                )
                .unwrap();
                let child = v8::Local::new(scope, &context);
                let policy = args
                    .get(2)
                    .boolean_value(scope)
                    .then(|| DocumentCodePolicy {
                        trusted_types: oryn_js::trusted_types::TrustedTypesConfig {
                            require_script: true,
                            deny_policy_creation: true,
                            ..Default::default()
                        },
                        allow_eval: false,
                    });
                if let Some(policy) = policy {
                    child
                        .get_slot::<V8BindingState>()
                        .unwrap()
                        .configure_trusted_types(policy.trusted_types.clone());
                    child.set_allow_generation_from_strings(policy.allow_eval);
                    child.set_slot(Rc::new(policy));
                }
                {
                    let child_scope = &mut v8::ContextScope::new(scope, child);
                    if args.get(1).boolean_value(child_scope) {
                        let environment = *child.get_slot::<EnvironmentDescriptor>().unwrap();
                        child
                            .get_slot::<V8BindingState>()
                            .unwrap()
                            .retire_execution();
                        retire_environment(child_scope, environment.id);
                    }
                }
                assert_eq!(scope.get_current_context(), caller);
                rv.set(child.global(scope).into());
            }
            let mut host = host();
            {
                let context = host.contexts.get(&RealmId(0)).unwrap().clone();
                v8::scope!(let scope, &mut host.isolate);
                let context = v8::Local::new(scope, &context);
                let scope = &mut v8::ContextScope::new(scope, context);
                let callback = v8::Function::new(scope, create_child).unwrap();
                let key = v8::String::new(scope, "createNativeChild").unwrap();
                context
                    .global(scope)
                    .set(scope, key.into(), callback.into());
            }
            assert_eq!(
                host.evaluate_string(
                    r#"
                globalThis.child=createNativeChild(false);
                const root=child.document.createElement('html');
                child.document.appendChild(root);
                root.textContent='synchronous';
                const opaque=createNativeChild(true);
                let denied=false;
                try { opaque.document; } catch (_) { denied=true; }
                [child.Array!==Array,root.ownerDocument===child.document,
                 child.document.documentElement.textContent==='synchronous',denied].every(Boolean)
            "#
                )
                .unwrap(),
                "true"
            );
            assert_eq!(host.realm_descriptor(RealmId(1)).unwrap().frame_id, Some(7));
            assert!(
                host.realm_descriptor(RealmId(2))
                    .unwrap()
                    .origin
                    .is_opaque()
            );
            host.execute("globalThis.retiredChild=createNativeChild(false,true)")
                .unwrap();
            assert_eq!(
                host.adopt_created_realms(),
                vec![RealmId(1), RealmId(2), RealmId(3)]
            );
            assert!(host.adopt_created_realms().is_empty());
            assert_eq!(host.realm_ids(), vec![RealmId(1), RealmId(2), RealmId(3)]);
            assert!(host.realm_execution_live(RealmId(1)));
            assert!(!host.realm_execution_live(RealmId(3)));
            host.activate_realm(RealmId(1)).unwrap();
            assert_eq!(
                host.evaluate_string("document.documentElement.textContent")
                    .unwrap(),
                "synchronous"
            );
            host.execute(
                "__oryn_timer_schedule(()=>{throw Error('retired timer fired')},0,false,[])",
            )
            .unwrap();
            host.activate_realm(RealmId(0)).unwrap();
            host.remove_realm(RealmId(1)).unwrap();
            assert_eq!(host.next_timer_delay(), None);
            host.remove_realm(RealmId(2)).unwrap();
            host.remove_realm(RealmId(3)).unwrap();
            assert!(host.realm_ids().is_empty());
            host.execute("globalThis.unadopted=createNativeChild(false)")
                .unwrap();
            host.remove_realm(RealmId(4)).unwrap();
            assert!(host.adopt_created_realms().is_empty());
            assert!(host.realm_descriptor(RealmId(4)).is_none());
            host.isolate.low_memory_notification();
            assert_eq!(
                host.evaluate_string("child.document.documentElement.textContent")
                    .unwrap(),
                "synchronous"
            );
            assert_eq!(host.evaluate_string(r#"(()=>{
                const protectedChild=createNativeChild(false,false,true);
                let htmlDenied=false, evalDenied=false, policyDenied=false;
                try { protectedChild.document.createElement('div').innerHTML='<p>raw</p>'; } catch (_) { htmlDenied=true; }
                try { protectedChild.eval('1'); } catch (_) { evalDenied=true; }
                try { protectedChild.trustedTypes.createPolicy('unapproved',{}); } catch (_) { policyDenied=true; }
                return htmlDenied && evalDenied && policyDenied && eval('2')===2;
            })()"#).unwrap(), "true");
            assert_eq!(host.adopt_created_realms(), vec![RealmId(5)]);
            host.remove_realm(RealmId(5)).unwrap();
        }

        #[test]
        fn native_context_plain_globals_keep_structural_semantics() {
            let mut host = host();
            let parent = host.contexts.get(&RealmId(0)).unwrap().clone();
            {
                v8::scope!(let scope, &mut host.isolate);
                let parent = v8::Local::new(scope, &parent);
                let child = v8::Context::new(scope, Default::default());
                child.set_security_token(parent.get_security_token(scope));
                let scope = &mut v8::ContextScope::new(scope, parent);
                let key = v8::String::new(scope, "ordinaryGlobal").unwrap();
                parent
                    .global(scope)
                    .set(scope, key.into(), child.global(scope).into());
            }
            assert_eq!(host.evaluate_string("(()=>{const w=ordinaryGlobal,p={};return [Reflect.setPrototypeOf(w,p),Object.getPrototypeOf(w)===p,Reflect.isExtensible(w),Reflect.preventExtensions(w),!Reflect.isExtensible(w),!Reflect.setPrototypeOf(w,{})].every(Boolean)})()").unwrap(), "true");
            assert_eq!(host.evaluate_string("(()=>{const o={},p={};return [Reflect.setPrototypeOf(o,p),Object.getPrototypeOf(o)===p,Reflect.preventExtensions(o),!Reflect.isExtensible(o)].every(Boolean)})()").unwrap(), "true");
        }

        #[test]
        fn native_context_window_structural_exotics_match_window_proxy_contract() {
            use oryn_js::v8_bindings::window_proxy as window;
            use v8::MapFnTo;
            fn unused(_: &mut v8::PinScope, _: v8::FunctionCallbackArguments, _: v8::ReturnValue) {}
            let mut host = host();
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<p>parent</p>")
                    .into_runtime()
                    .unwrap(),
            ));
            host.install_dom(dom, 1).unwrap();
            let parent = host.contexts.get(&RealmId(0)).unwrap().clone();
            let (child, state) = {
                v8::scope!(let scope, &mut host.isolate);
                let parent = v8::Local::new(scope, &parent);
                let template = v8::ObjectTemplate::new(scope);
                window::configure_template(scope, template);
                let child = v8::Context::new(
                    scope,
                    v8::ContextOptions {
                        global_template: Some(template),
                        ..Default::default()
                    },
                );
                child.set_security_token(v8::String::new(scope, "foreign").unwrap().into());
                let state = window::install_context(
                    child,
                    window::WindowCrossOriginCallbacks {
                        properties: [unused.map_fn_to(); 13],
                        location_setter: unused.map_fn_to(),
                    },
                );
                state.set_children(vec![v8::Global::new(scope, parent.global(scope))]);
                let scope = &mut v8::ContextScope::new(scope, parent);
                let key = v8::String::new(scope, "structuralWindow").unwrap();
                parent
                    .global(scope)
                    .set(scope, key.into(), child.global(scope).into());
                (v8::Global::new(scope, child), state)
            };
            let foreign = host.evaluate_string("[()=>Object.getPrototypeOf(structuralWindow)===null,()=>Reflect.isExtensible(structuralWindow),()=>Reflect.preventExtensions(structuralWindow),()=>Reflect.setPrototypeOf(structuralWindow,null),()=>Reflect.setPrototypeOf(structuralWindow,{})].map(f=>{try{return String(f())}catch(e){return e.name}}).join('|')").unwrap();
            let foreign_objects = host.evaluate_string("[()=>Object.setPrototypeOf(structuralWindow,null)===structuralWindow,()=>Object.setPrototypeOf(structuralWindow,{}),()=>Object.preventExtensions(structuralWindow),()=>Object.seal(structuralWindow),()=>Object.freeze(structuralWindow),()=>structuralWindow.document].map(f=>{try{return String(f())}catch(e){return e.name}}).join('|')").unwrap();
            {
                v8::scope!(let scope, &mut host.isolate);
                let parent = v8::Local::new(scope, &parent);
                let child = v8::Local::new(scope, &child);
                child.set_security_token(parent.get_security_token(scope));
            }
            assert_eq!(host.evaluate_string("(()=>{const w=structuralWindow,d=Object.getOwnPropertyDescriptor(w,'0');return [w[0]===globalThis,w[1]===undefined,d.value===globalThis,d.enumerable,!d.writable,d.configurable,'0' in w,!('1' in w),!Reflect.set(w,'0',1),!Reflect.set(w,'1',1),!Reflect.defineProperty(w,'0',{value:1}),!Reflect.defineProperty(w,'1',{value:1}),!Reflect.deleteProperty(w,'0'),Reflect.deleteProperty(w,'1')].every(Boolean)})()").unwrap(), "true");
            assert_eq!(host.evaluate_string("[()=>{ 'use strict'; structuralWindow[0]=1 },()=>Object.defineProperty(structuralWindow,'0',{value:1})].map(f=>{try{f();return 'accepted'}catch(e){return e.name}}).join('|')").unwrap(), "TypeError|TypeError");
            state.set_children(Vec::new());
            assert_eq!(host.evaluate_string("structuralWindow[0]===undefined && Reflect.deleteProperty(structuralWindow,'0')").unwrap(), "true");
            let local = host.evaluate_string("[()=>Reflect.setPrototypeOf(structuralWindow,Object.getPrototypeOf(structuralWindow)),()=>Reflect.setPrototypeOf(structuralWindow,{}),()=>Reflect.isExtensible(structuralWindow),()=>Reflect.preventExtensions(structuralWindow)].map(f=>{try{return String(f())}catch(e){return e.name}}).join('|')").unwrap();
            assert_eq!(
                format!("{foreign};{local}"),
                "true|true|false|true|false;true|false|true|false"
            );
            assert_eq!(
                foreign_objects,
                "true|TypeError|TypeError|TypeError|TypeError|SecurityError"
            );
            host.execute(
                "var retainedStructuralWindow=structuralWindow;structuralWindow.retiredValue=17",
            )
            .unwrap();
            let replacement = {
                v8::scope!(let scope, &mut host.isolate);
                let child = v8::Local::new(scope, &child);
                let global = child.global(scope);
                oryn_js::v8_context_bridge::detach_global(scope, child);
                let template = v8::ObjectTemplate::new(scope);
                window::configure_template(scope, template);
                let replacement = v8::Context::new(
                    scope,
                    v8::ContextOptions {
                        global_template: Some(template),
                        global_object: Some(global.into()),
                        ..Default::default()
                    },
                );
                replacement.set_security_token(
                    v8::String::new(scope, "foreign-replacement")
                        .unwrap()
                        .into(),
                );
                window::install_context(
                    replacement,
                    window::WindowCrossOriginCallbacks {
                        properties: [unused.map_fn_to(); 13],
                        location_setter: unused.map_fn_to(),
                    },
                );
                v8::Global::new(scope, replacement)
            };
            assert_eq!(host.evaluate_string("retainedStructuralWindow===structuralWindow && Object.getPrototypeOf(structuralWindow)===null && Reflect.isExtensible(structuralWindow) && !Reflect.preventExtensions(structuralWindow) && Reflect.setPrototypeOf(structuralWindow,null) && !Reflect.setPrototypeOf(structuralWindow,{})").unwrap(), "true");
            {
                v8::scope!(let scope, &mut host.isolate);
                let parent = v8::Local::new(scope, &parent);
                let replacement = v8::Local::new(scope, &replacement);
                replacement.set_security_token(parent.get_security_token(scope));
            }
            assert_eq!(host.evaluate_string("structuralWindow.retiredValue===undefined && !Reflect.setPrototypeOf(structuralWindow,{}) && !Reflect.preventExtensions(structuralWindow)").unwrap(), "true");
        }

        #[test]
        fn native_context_window_descriptors_use_native_callbacks_and_caller_realm() {
            use oryn_js::v8_bindings::window_proxy as window;
            use v8::MapFnTo;
            fn original_window(
                scope: &mut v8::PinScope,
                args: v8::FunctionCallbackArguments,
                mut rv: v8::ReturnValue,
            ) {
                let context = window::callback_context(scope, &args).unwrap();
                rv.set(context.global(scope).into());
            }
            fn marker(
                scope: &mut v8::PinScope,
                args: v8::FunctionCallbackArguments,
                mut rv: v8::ReturnValue,
            ) {
                let context = window::callback_context(scope, &args).unwrap();
                let marker = context.get_slot::<String>().unwrap();
                rv.set(v8::String::new(scope, &marker).unwrap().into());
            }
            let mut host = host();
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<p>parent</p>")
                    .into_runtime()
                    .unwrap(),
            ));
            host.install_dom(dom, 1).unwrap();
            let parent = host.contexts.get(&RealmId(0)).unwrap().clone();
            let child = {
                v8::scope!(let scope, &mut host.isolate);
                let parent = v8::Local::new(scope, &parent);
                let template = v8::ObjectTemplate::new(scope);
                window::configure_template(scope, template);
                let child = v8::Context::new(
                    scope,
                    v8::ContextOptions {
                        global_template: Some(template),
                        ..Default::default()
                    },
                );
                child.set_security_token(v8::String::new(scope, "foreign").unwrap().into());
                child.set_slot(Rc::new("original".to_owned()));
                let mut callbacks = [original_window.map_fn_to(); 13];
                callbacks[12] = marker.map_fn_to();
                let state = window::install_context(
                    child,
                    window::WindowCrossOriginCallbacks {
                        properties: callbacks,
                        location_setter: marker.map_fn_to(),
                    },
                );
                state.set_children(vec![v8::Global::new(scope, parent.global(scope))]);
                {
                    let scope = &mut v8::ContextScope::new(scope, child);
                    let source = v8::String::new(scope, "globalThis.postMessage=function(){return 'author replacement'};globalThis.close=23;Object.defineProperty(globalThis,'focus',{configurable:true,get(){throw new Error('author getter must not run')}})").unwrap();
                    v8::Script::compile(scope, source, None)
                        .unwrap()
                        .run(scope)
                        .unwrap();
                }
                {
                    let scope = &mut v8::ContextScope::new(scope, parent);
                    let key = v8::String::new(scope, "foreignWindow").unwrap();
                    parent
                        .global(scope)
                        .set(scope, key.into(), child.global(scope).into());
                }
                v8::Global::new(scope, child)
            };
            host.execute(
                "globalThis.WeakMap=function(){throw new Error('author WeakMap must not run')}",
            )
            .unwrap();
            assert_eq!(
                host.evaluate_string("foreignWindow.close+'|'+String(foreignWindow.focus)")
                    .unwrap(),
                "23|undefined"
            );
            assert_eq!(host.evaluate_string("foreignWindow.window===foreignWindow && foreignWindow.self===foreignWindow && foreignWindow[0]===globalThis").unwrap(), "true");
            assert_eq!(
                host.evaluate_string("foreignWindow.postMessage()").unwrap(),
                "original"
            );
            assert_eq!(host.evaluate_string("(()=>{const a=Object.getOwnPropertyDescriptor(foreignWindow,'window'),b=Object.getOwnPropertyDescriptor(foreignWindow,'window');globalThis.savedGetter=a.get;globalThis.savedMethod=foreignWindow.postMessage;return a.get===b.get && a.get instanceof Function && !a.enumerable && a.configurable && a.set===undefined && a.get()===foreignWindow})()").unwrap(), "true");
            {
                v8::scope!(let scope, &mut host.isolate);
                let context = v8::Local::new(scope, &child);
                let scope = &mut v8::ContextScope::new(scope, context);
                let source = v8::String::new(scope, "close=99;postMessage=5").unwrap();
                v8::Script::compile(scope, source, None)
                    .unwrap()
                    .run(scope)
                    .unwrap();
            }
            host.isolate.low_memory_notification();
            assert_eq!(host.evaluate_string("foreignWindow.close+'|'+String(foreignWindow.postMessage===savedMethod)+'|'+String(Object.getOwnPropertyDescriptor(foreignWindow,'window').get===savedGetter)").unwrap(), "23|true|true");
            assert_eq!(host.evaluate_string("(()=>{const d=Object.getOwnPropertyDescriptor(foreignWindow,'close');return d.value===23 && !d.writable && !d.enumerable && d.configurable})()").unwrap(), "true");
            assert_eq!(host.evaluate_string("foreignWindow.then===undefined && foreignWindow[Symbol.toStringTag]===undefined && foreignWindow[Symbol.hasInstance]===undefined && foreignWindow[Symbol.isConcatSpreadable]===undefined").unwrap(), "true");
            assert_eq!(host.evaluate_string("(()=>{const d=Object.getOwnPropertyDescriptor(foreignWindow,'then');return d.value===undefined && !d.writable && !d.enumerable && d.configurable})()").unwrap(), "true");
            assert_eq!(host.evaluate_string("Reflect.ownKeys(foreignWindow).map(k=>typeof k==='symbol'?k.toString():k).join('|')").unwrap(), "0|window|self|location|close|closed|focus|blur|frames|length|top|opener|parent|postMessage|then|Symbol(Symbol.toStringTag)|Symbol(Symbol.hasInstance)|Symbol(Symbol.isConcatSpreadable)");
            assert_eq!(host.evaluate_string("(()=>{const Original=DOMException;globalThis.DOMException=function(){throw 1};return [()=>foreignWindow.secret,()=>foreignWindow[1],()=>{foreignWindow.window=1},()=>Object.defineProperty(foreignWindow,'window',{value:1}),()=>{delete foreignWindow.window}].map(action=>{try{action();return 'accepted'}catch(e){return e.name+':'+String(e instanceof Original)}}).join('|')})()").unwrap(), "SecurityError:true|SecurityError:true|SecurityError:true|SecurityError:true|SecurityError:true");
            let old_weak = {
                v8::scope!(let scope, &mut host.isolate);
                let old = v8::Local::new(scope, &child);
                let proxy = old.global(scope);
                oryn_js::v8_context_bridge::detach_global(scope, old);
                let replacement = v8::Context::new(
                    scope,
                    v8::ContextOptions {
                        global_object: Some(proxy.into()),
                        ..Default::default()
                    },
                );
                replacement.set_slot(Rc::new("replacement".to_owned()));
                v8::Weak::new(scope, old)
            };
            drop(child);
            host.isolate.low_memory_notification();
            // The callable's private context remains the original Window realm.
            assert_eq!(host.evaluate_string("savedMethod()").unwrap(), "original");
            host.execute("savedGetter=null;savedMethod=null;foreignWindow=null")
                .unwrap();
            host.isolate.low_memory_notification();
            v8::scope!(let scope, &mut host.isolate);
            assert!(
                old_weak.to_local(scope).is_none(),
                "descriptor cache must not root a retired Window"
            );
        }

        #[test]
        fn native_context_cross_origin_interceptors_preserve_normal_local_access() {
            use v8::MapFnTo;
            let mut host = host();
            let parent = host.contexts.get(&RealmId(0)).unwrap().clone();
            let child = {
                v8::scope!(let scope, &mut host.isolate);
                let parent = v8::Local::new(scope, &parent);
                let template = v8::ObjectTemplate::new(scope);
                let getter = |scope: &mut v8::PinScope,
                              key: v8::Local<v8::Name>,
                              _: v8::PropertyCallbackArguments,
                              mut rv: v8::ReturnValue| {
                    if key.to_rust_string_lossy(scope) == "exposed" {
                        rv.set(v8::String::new(scope, "native").unwrap().into());
                    } else {
                        let message = v8::String::new(scope, "denied by native handler").unwrap();
                        let error = v8::Exception::type_error(scope, message);
                        scope.throw_exception(error);
                    }
                    v8::Intercepted::kYes
                };
                let indexed = |_: &mut v8::PinScope,
                               index: u32,
                               _: v8::PropertyCallbackArguments,
                               mut rv: v8::ReturnValue| {
                    rv.set_uint32(100 + index);
                    v8::Intercepted::kYes
                };
                oryn_js::v8_context_bridge::set_cross_context_handlers(
                    scope,
                    template,
                    &oryn_js::v8_context_bridge::CrossContextHandlers {
                        named_getter: Some(getter.map_fn_to()),
                        indexed_getter: Some(indexed.map_fn_to()),
                        ..Default::default()
                    },
                );
                let child = v8::Context::new(
                    scope,
                    v8::ContextOptions {
                        global_template: Some(template),
                        ..Default::default()
                    },
                );
                child.set_security_token(parent.get_security_token(scope));
                {
                    let scope = &mut v8::ContextScope::new(scope, child);
                    let source = v8::String::new(
                        scope,
                        "var exposed='author';var secret=42;globalThis[3]=7",
                    )
                    .unwrap();
                    v8::Script::compile(scope, source, None)
                        .unwrap()
                        .run(scope)
                        .unwrap();
                }
                {
                    let scope = &mut v8::ContextScope::new(scope, parent);
                    let key = v8::String::new(scope, "foreignWindow").unwrap();
                    parent
                        .global(scope)
                        .set(scope, key.into(), child.global(scope).into());
                }
                v8::Global::new(scope, child)
            };
            assert_eq!(
                host.evaluate_string(
                    "foreignWindow.exposed+'|'+foreignWindow.secret+'|'+foreignWindow[3]"
                )
                .unwrap(),
                "author|42|7"
            );
            {
                v8::scope!(let scope, &mut host.isolate);
                let child = v8::Local::new(scope, &child);
                child.set_security_token(v8::String::new(scope, "opaque:other").unwrap().into());
            }
            assert_eq!(
                host.evaluate_string("foreignWindow.exposed+'|'+foreignWindow[3]")
                    .unwrap(),
                "native|103"
            );
            assert!(host.evaluate_string("foreignWindow.secret").is_err());
            assert!(host.evaluate_string("foreignWindow.secret=0").is_err());
            {
                v8::scope!(let scope, &mut host.isolate);
                let parent = v8::Local::new(scope, &parent);
                let child = v8::Local::new(scope, &child);
                child.set_security_token(parent.get_security_token(scope));
            }
            assert_eq!(
                host.evaluate_string(
                    "foreignWindow.exposed+'|'+foreignWindow.secret+'|'+foreignWindow[3]"
                )
                .unwrap(),
                "author|42|7"
            );
        }

        #[test]
        fn isolate_watchdog_reuses_one_thread_and_disarmed_deadlines_cannot_poison_later_scripts() {
            let mut host = host();
            let thread = host.watchdog.thread.as_ref().unwrap().thread().id();
            for _ in 0..500 {
                let deadline = ExecutionDeadline::arm(&host.isolate, Duration::ZERO);
                std::thread::yield_now();
                drop(deadline);
                assert_eq!(host.evaluate_string("1+1").unwrap(), "2");
            }
            assert_eq!(host.watchdog.thread.as_ref().unwrap().thread().id(), thread);
            assert!(
                host.watchdog
                    .state
                    .schedule
                    .lock()
                    .unwrap()
                    .active
                    .is_empty()
            );
        }

        #[test]
        fn isolate_watchdog_exhaustion_is_fail_closed_and_ids_never_wrap() {
            let host = host();
            let guards = (0..64)
                .map(|_| ExecutionDeadline::arm(&host.isolate, Duration::from_secs(30)))
                .collect::<Vec<_>>();
            let overflow = ExecutionDeadline::arm(&host.isolate, Duration::from_secs(30));
            assert!(overflow.expired());
            assert!(overflow.id.is_none());
            drop(overflow);
            drop(guards);
            host.watchdog.state.schedule.lock().unwrap().next_id = u64::MAX;
            let overflow = ExecutionDeadline::arm(&host.isolate, Duration::from_secs(30));
            assert!(overflow.expired());
            assert!(overflow.id.is_none());
            assert_eq!(
                host.watchdog.state.schedule.lock().unwrap().next_id,
                u64::MAX
            );
        }

        #[test]
        fn isolate_watchdog_preserves_the_earliest_nested_deadline() {
            let mut host = host();
            let outer = ExecutionDeadline::arm(&host.isolate, Duration::from_millis(25));
            let inner = ExecutionDeadline::arm(&host.isolate, Duration::from_secs(30));
            assert!(matches!(
                host.execute("for(;;){}"),
                Err(HostError::LimitExceeded { .. })
            ));
            assert!(outer.expired());
            assert!(!inner.expired());
            drop(inner);
            drop(outer);
            assert_eq!(host.evaluate_string("1+1").unwrap(), "2");
        }

        #[test]
        fn isolate_watchdog_is_joined_before_host_destruction_returns() {
            let host = host();
            let state = host.watchdog.state.clone();
            drop(host);
            assert!(state.schedule.lock().unwrap().shutdown);
            assert_eq!(Arc::strong_count(&state), 1);
        }

        #[test]
        fn timer_exception_is_reported_in_callback_realm_and_peer_tasks_continue() {
            let mut host = host();
            host.install_dom(Rc::new(RefCell::new(oryn_dom::DomRuntime::new())), 1)
                .unwrap();
            let frame = host
                .create_realm_with_dom(
                    "https://example.test",
                    1,
                    Some(7),
                    Rc::new(RefCell::new(oryn_dom::DomRuntime::new())),
                )
                .unwrap();
            expose_child(&mut host, frame);
            host.execute("globalThis.errors=0;globalThis.peer=0;onerror=()=>errors++;child.errors=0;child.onerror=function(){child.errors++;return false};__oryn_timer_schedule(new child.Function('error','throw error'),0,false,[new Error('timer')]);__oryn_timer_schedule(()=>peer++,0,false,[])").unwrap();
            host.drain_scheduled_timers(false).unwrap();
            assert_eq!(
                host.evaluate_string("[errors,child.errors,peer].join(',')")
                    .unwrap(),
                "0,1,1"
            );
            host.execute("__oryn_timer_schedule(new child.Function(\"'use strict';globalThis.timerThis=this\"),0,false,[])").unwrap();
            host.drain_scheduled_timers(false).unwrap();
            assert_eq!(
                host.evaluate_string("child.timerThis===globalThis")
                    .unwrap(),
                "true"
            );
        }

        #[test]
        fn native_context_incumbent_tracks_the_calling_script() {
            let mut host = host();
            let frame = host
                .create_realm("https://example.test", 1, Some(7))
                .unwrap();
            expose_child(&mut host, frame);
            {
                let context = host.contexts.get(&frame).unwrap().clone();
                v8::scope!(let scope, &mut host.isolate);
                let context = v8::Local::new(scope, &context);
                let scope = &mut v8::ContextScope::new(scope, context);
                let callback = v8::Function::new(scope, inspect_incumbent).unwrap();
                let key = v8::String::new(scope, "inspectIncumbent").unwrap();
                context
                    .global(scope)
                    .set(scope, key.into(), callback.into());
            }
            assert_eq!(
                host.evaluate_string("child.inspectIncumbent()").unwrap(),
                "1:0"
            );
            assert_eq!(
                host.evaluate_string("child.eval('inspectIncumbent()')")
                    .unwrap(),
                "1:1"
            );
            host.execute("globalThis.result='pending'; child.eval('Promise.resolve().then(()=>globalThis.result=inspectIncumbent())')").unwrap();
            assert_eq!(host.evaluate_string("child.result").unwrap(), "1:1");
        }

        #[test]
        fn native_context_replacement_preserves_proxy_and_retained_environment() {
            let mut host = host();
            let frame = host
                .create_realm("https://example.test", 1, Some(7))
                .unwrap();
            host.activate_realm(frame).unwrap();
            host.execute("var marker='old'; var document={generation:1}; function readOld(){return marker+':'+document.generation}").unwrap();
            expose_child(&mut host, frame);
            host.activate_realm(RealmId(0)).unwrap();
            host.execute("var held=child;var readOld=child.readOld;var oldDocument=child.document")
                .unwrap();
            let replacement = host
                .replace_frame_realm(frame, "https://example.test", 2)
                .unwrap();
            assert_eq!(host.active_realm(), RealmId(0));
            assert!(host.realm_descriptor(frame).is_none());
            assert_eq!(
                host.realm_descriptor(replacement)
                    .unwrap()
                    .document_generation,
                2
            );
            host.activate_realm(replacement).unwrap();
            assert_eq!(host.evaluate_string("typeof marker").unwrap(), "undefined");
            host.execute("var marker='new';var document={generation:2}")
                .unwrap();
            host.activate_realm(RealmId(0)).unwrap();
            assert_eq!(host.evaluate_string("held===child && child===child.globalThis && child.marker==='new' && child.document!==oldDocument && readOld()==='old:1'").unwrap(), "true");
            let foreign = host
                .replace_frame_realm(replacement, "https://foreign.test", 3)
                .unwrap();
            host.activate_realm(foreign).unwrap();
            host.execute("var secret=42").unwrap();
            host.activate_realm(RealmId(0)).unwrap();
            assert_eq!(host.evaluate_string("held===child").unwrap(), "true");
            assert!(host.evaluate_string("child.secret").is_err());
            assert_eq!(host.evaluate_string("readOld()").unwrap(), "old:1");
            host.activate_realm(foreign).unwrap();
            let returned = host
                .replace_frame_realm(foreign, "https://example.test", 4)
                .unwrap();
            assert_eq!(host.active_realm(), returned);
            host.execute("var marker='returned'").unwrap();
            host.activate_realm(RealmId(0)).unwrap();
            assert_eq!(host.evaluate_string("held===child && child.marker==='returned' && typeof child.secret==='undefined' && readOld()==='old:1'").unwrap(), "true");
        }

        #[test]
        fn native_context_navigation_checks_borrowed_dom_operations() {
            let mut host = host();
            let dom = |html| {
                Rc::new(RefCell::new(
                    oryn_dom::html::parse(html).into_runtime().unwrap(),
                ))
            };
            host.install_dom(dom("<body id='parent'>"), 1).unwrap();
            let frame = host
                .create_realm_with_dom("https://example.test", 1, Some(7), dom("<body id='old'>"))
                .unwrap();
            expose_child(&mut host, frame);
            host.execute("globalThis.savedAdd=child.addEventListener;globalThis.savedDispatch=child.dispatchEvent;globalThis.savedRemove=child.removeEventListener;globalThis.savedOnError=Object.getOwnPropertyDescriptor(child,'onerror');globalThis.SavedDOMException=child.DOMException;globalThis.oldBody=child.document.body;globalThis.payload=new Event('probe');savedAdd.call(child,'probe',()=>{});globalThis.before=child===child.globalThis").unwrap();
            let next = host
                .replace_frame_realm(frame, "https://foreign.test", 2)
                .unwrap();
            host.activate_realm(next).unwrap();
            host.install_dom(dom("<body id='foreign'>"), 2).unwrap();
            host.execute("globalThis.calls=0;addEventListener('probe',()=>calls++)")
                .unwrap();
            host.activate_realm(RealmId(0)).unwrap();
            assert_eq!(host.evaluate_string("before").unwrap(), "true");
            assert!(host.evaluate_string("child.document").is_err());
            host.execute(r#"
                globalThis.converted = false;
                globalThis.errorResults = [];
                const type = {toString(){converted=true;return 'probe'}};
                for (const call of [
                    () => savedAdd.call(child, type, () => {}),
                    () => savedRemove.call(child, type, () => {}),
                    () => savedAdd.call(child),
                    () => savedDispatch.call(child, payload),
                    () => savedOnError.get.call(child),
                    () => savedOnError.set.call(child, () => {})
                ]) {
                    try { call(); errorResults.push('accepted'); }
                    catch (error) { errorResults.push(error.name+':'+(error instanceof SavedDOMException)); }
                }
            "#).unwrap();
            assert_eq!(host.evaluate_string("!converted && errorResults.length===6 && errorResults.every(value=>value==='SecurityError:true')").unwrap(), "true");
            // Retaining an old same-origin document does not grant access to
            // the replacement document, and remains a valid detached DOM reference.
            assert_eq!(host.evaluate_string("oldBody.id").unwrap(), "old");
            host.activate_realm(next).unwrap();
            assert_eq!(host.evaluate_string("calls").unwrap(), "0");
        }

        #[test]
        fn message_channel_transfer_uses_destination_realm_and_retires_on_navigation() {
            let mut host = host();
            let dom = || {
                Rc::new(RefCell::new(
                    oryn_dom::html::parse("<body>").into_runtime().unwrap(),
                ))
            };
            host.install_dom(dom(), 1).unwrap();
            let frame = host
                .create_realm_with_dom("https://example.test", 1, Some(7), dom())
                .unwrap();
            expose_child(&mut host, frame);
            host.execute(
                r#"
              globalThis.channel = new MessageChannel();
              channel.port2.postMessage({value:'queued'});
              child.moved = child.structuredClone(channel.port1, {transfer:[channel.port1]});
              channel.port1.close();
              globalThis.retained=[]; child.results=retained;
            "#,
            )
            .unwrap();
            host.activate_realm(frame).unwrap();
            host.execute(r#"
              moved.onmessage = event => results.push(event instanceof MessageEvent && event.data instanceof Object && event.data.value);
            "#).unwrap();
            host.drain_async_fetches(false).unwrap();
            host.activate_realm(RealmId(0)).unwrap();
            assert_eq!(
                host.evaluate_string("JSON.stringify(retained)").unwrap(),
                r#"["queued"]"#
            );
            host.execute("channel.port2.postMessage({value:'must retire'})")
                .unwrap();
            let replacement = host
                .replace_frame_realm(frame, "https://foreign.test", 2)
                .unwrap();
            host.activate_realm(replacement).unwrap();
            host.install_dom(dom(), 2).unwrap();
            host.drain_async_fetches(false).unwrap();
            host.activate_realm(RealmId(0)).unwrap();
            host.execute("channel.port2.postMessage({value:'after retirement'})")
                .unwrap();
            host.drain_async_fetches(false).unwrap();
            assert_eq!(
                host.evaluate_string("JSON.stringify(retained)").unwrap(),
                r#"["queued"]"#
            );
        }

        #[test]
        fn native_context_dom_exception_ignores_replaced_global_constructor() {
            let mut host = host();
            let dom = Rc::new(RefCell::new(
                oryn_dom::html::parse("<body>").into_runtime().unwrap(),
            ));
            host.install_dom(dom, 1).unwrap();
            host.execute(r#"
                globalThis.OriginalException=DOMException;
                globalThis.constructorCalls=0;
                globalThis.DOMException=function(){constructorCalls++;return {fake:true}};
                globalThis.correct=false;
                try { document.createElement('invalid name'); }
                catch(error) { correct=error instanceof OriginalException && error.name==='InvalidCharacterError'; }
            "#).unwrap();
            assert_eq!(
                host.evaluate_string("correct && constructorCalls===0")
                    .unwrap(),
                "true"
            );
        }

        #[test]
        fn native_context_replacement_rejects_non_frame_without_mutation() {
            let mut host = host();
            host.execute("var marker='unchanged'").unwrap();
            assert!(
                host.replace_frame_realm(RealmId(0), "https://foreign.test", 2)
                    .is_err()
            );
            assert!(
                host.replace_frame_realm(RealmId(999), "https://foreign.test", 2)
                    .is_err()
            );
            assert_eq!(host.evaluate_string("marker").unwrap(), "unchanged");
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
    fn timer_delay_uses_webidl_long_and_preserves_initial_interval_order() {
        let mut host = RawV8Host::new();
        host.execute("globalThis.order=[];globalThis.interval=__oryn_timer_schedule(()=>{order.push('interval');__oryn_timer_cancel(interval)},4294967296,true,[]);__oryn_timer_schedule(()=>order.push('timeout'),4294967296,false,[])").unwrap();
        host.drain_scheduled_timers(false).unwrap();
        assert_eq!(
            host.evaluate_string("order.join(',')").unwrap(),
            "interval,timeout"
        );
        assert_eq!(host.next_timer_delay(), None);
        host.execute("__oryn_timer_schedule(()=>order.push('negative'),2147483648,false,[])")
            .unwrap();
        host.drain_scheduled_timers(false).unwrap();
        assert_eq!(
            host.evaluate_string("order.join(',')").unwrap(),
            "interval,timeout,negative"
        );
        assert_eq!(
            host.evaluate_string(
                "try{__oryn_timer_schedule(()=>{},Symbol(),false,[]);'missed'}catch(e){e.name}"
            )
            .unwrap(),
            "TypeError"
        );
        assert_eq!(host.next_timer_delay(), None);
    }

    #[test]
    fn timer_watchdog_reports_limit_and_allows_followup_execution() {
        let mut host = RawV8Host::new_with_wall_limit(Duration::from_millis(25));
        host.execute("__oryn_timer_schedule(() => { for (;;) {} }, 0, false, [])")
            .unwrap();
        let result = host.drain_scheduled_timers(true);
        assert!(
            matches!(result, Err(HostError::LimitExceeded { .. })),
            "{result:?}"
        );
        assert_eq!(host.evaluate_string("1 + 1").unwrap(), "2");
        assert_eq!(host.drain_scheduled_timers(false).unwrap(), 0);
    }

    #[test]
    fn frame_retirement_cancels_its_timers_without_cancelling_sibling_timers() {
        let mut host = RawV8Host::new();
        let root_timer = host
            .evaluate_string("globalThis.fired=0;__oryn_timer_schedule(()=>fired++,0,false,[])")
            .unwrap();
        let frame = host
            .create_realm("https://example.test", 7, Some(1))
            .unwrap();
        host.activate_realm(frame).unwrap();
        host.execute(&format!("__oryn_timer_cancel({root_timer});__oryn_timer_schedule(()=>{{throw Error('retired timeout')}},0,false,[]);__oryn_timer_schedule(()=>{{throw Error('retired interval')}},100000,true,[])" )).unwrap();
        host.remove_realm(frame).unwrap();
        assert_eq!(host.drain_scheduled_timers(false).unwrap(), 1);
        assert_eq!(host.evaluate_string("fired").unwrap(), "1");
        assert_eq!(host.next_timer_delay(), None);
    }

    #[test]
    fn frame_retirement_discards_late_fetch_completion_without_running_reactions() {
        let mut host = RawV8Host::new_with_network(
            SharedNetworkBroker::new(SlowFixtureBroker),
            Url::parse("https://example.test/").unwrap(),
        );
        let frame = host
            .create_realm("https://example.test", 7, Some(1))
            .unwrap();
        host.activate_realm(frame).unwrap();
        host.execute("__oryn_fetch_async('/item','GET','','[]','base64','same-origin').then(()=>{throw Error('retired fetch reaction')})").unwrap();
        host.remove_realm(frame).unwrap();
        assert!(!host.has_pending_async_fetches());
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(host.drain_async_fetches(false).unwrap(), 0);
        host.execute("globalThis.result='pending';__oryn_fetch_async('/item','GET','','[]','base64','same-origin').then(()=>result='live')").unwrap();
        assert_eq!(host.drain_async_fetches(true).unwrap(), 1);
        assert_eq!(host.evaluate_string("result").unwrap(), "live");
    }

    #[test]
    fn evaluation_reports_thrown_values_without_a_stack_trace() {
        let mut host = RawV8Host::new();
        for (source, expected) in [
            ("throw 'diagnostic sentinel'", "diagnostic sentinel"),
            ("throw 42", "42"),
            ("throw null", "null"),
            ("throw undefined", "undefined"),
        ] {
            let error = host.evaluate_string(source).unwrap_err();
            assert!(
                matches!(error, HostError::Execution(ref detail) if detail == expected),
                "{error:?}"
            );
        }
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
        let frame = host
            .create_realm("https://example.test", 4, Some(7))
            .unwrap();
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
    fn observer_microtasks_follow_native_mutations_and_remain_deadline_bounded() {
        let dom = Rc::new(RefCell::new(
            oryn_dom::html::parse("<main id='root'></main>")
                .into_runtime()
                .unwrap(),
        ));
        let mut host = RawV8Host::new_with_wall_limit(Duration::from_millis(100));
        host.install_dom(dom.clone(), 1).unwrap();
        host.execute("globalThis.order=[]; globalThis.observer=new MutationObserver(()=>order.push('observer')); observer.observe(document.getElementById('root'),{attributes:true});").unwrap();
        let root = dom.borrow().find_by_id("root").unwrap();
        dom.borrow_mut()
            .set_attribute(root, "changed", "yes")
            .unwrap();
        host.execute("Promise.resolve().then(()=>order.push('promise'))")
            .unwrap();
        assert_eq!(
            host.evaluate_string("order.join('|')").unwrap(),
            "observer|promise"
        );
        host.execute("observer.disconnect(); globalThis.loopObserver=new MutationObserver(()=>document.getElementById('root').setAttribute('loop','yes')); loopObserver.observe(document.getElementById('root'),{attributes:true});").unwrap();
        dom.borrow_mut()
            .set_attribute(root, "loop", "start")
            .unwrap();
        let result = host.run_dom_observer_checkpoint();
        assert!(
            matches!(result, Err(HostError::LimitExceeded { .. })),
            "{result:?}"
        );
        // The host remains callable after the bounded failure.
        assert_eq!(
            host.evaluate_string("loopObserver.disconnect(); 1 + 1")
                .unwrap(),
            "2"
        );
        host.execute("globalThis.recovered=false;new MutationObserver(()=>recovered=true).observe(document.getElementById('root'),{attributes:true});document.getElementById('root').setAttribute('recovered','yes')").unwrap();
        assert_eq!(host.evaluate_string("String(recovered)").unwrap(), "true");
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
    fn shared_worker_connect_event_exposes_its_message_port() {
        let mut host = RawV8Host::new();
        let source = serde_json::to_string(
            "addEventListener('connect',event=>event.ports[0].postMessage({ports:event.ports.length,source:event.source===event.ports[0]}))",
        )
        .unwrap();
        let id = host
            .evaluate_string(&format!(
                "__oryn_worker_create({source},'https://example.test/worker.js',false,'sharedworker')"
            ))
            .unwrap();
        let messages = host
            .evaluate_string(&format!("__oryn_worker_take({id})"))
            .unwrap();
        let messages: Vec<String> = serde_json::from_str(&messages).unwrap();
        let payload: serde_json::Value = serde_json::from_str(&messages[0]).unwrap();
        assert_eq!(payload, serde_json::json!({"ports": 1, "source": true}));
    }

    #[test]
    fn worker_globals_preserve_native_event_target_dispatch() {
        let mut host = RawV8Host::new();
        for kind in ["dedicatedworker", "sharedworker", "serviceworker"] {
            let source = serde_json::to_string(
                r#"
                const order = [];
                const identity = ['addEventListener', 'removeEventListener', 'dispatchEvent']
                  .every(name => self[name] === EventTarget.prototype[name]);
                function removed() { order.push('removed'); }
                addEventListener('probe', () => {
                  order.push('once');
                  removeEventListener('probe', removed);
                  dispatchEvent(new Event('probe'));
                }, {once: true});
                addEventListener('probe', removed);
                addEventListener('probe', event => {
                  order.push('listener:' + (event.target === self && event.currentTarget === self));
                  event.preventDefault();
                });
                const event = new Event('probe', {cancelable: true});
                const accepted = dispatchEvent(event);
                let invalidReceiver = false;
                try { EventTarget.prototype.addEventListener.call({}, 'probe', () => {}); }
                catch (error) { invalidReceiver = error instanceof TypeError; }
                postMessage({identity, order, accepted, canceled: event.defaultPrevented,
                  cleaned: event.currentTarget === null, invalidReceiver});
                "#,
            )
            .unwrap();
            let id = host
                .evaluate_string(&format!(
                    "__oryn_worker_create({source},'https://example.test/worker.js',false,'{kind}')"
                ))
                .unwrap();
            let messages = host
                .evaluate_string(&format!("__oryn_worker_take({id})"))
                .unwrap();
            let messages: Vec<String> = serde_json::from_str(&messages).unwrap();
            assert_eq!(messages.len(), 1, "{kind}: {messages:?}");
            let payload: serde_json::Value = serde_json::from_str(&messages[0]).unwrap();
            assert_eq!(
                payload,
                serde_json::json!({
                    "identity": true,
                    "order": ["once", "listener:true", "listener:true"],
                    "accepted": false,
                    "canceled": true,
                    "cleaned": true,
                    "invalidReceiver": true
                }),
                "{kind}"
            );
            host.execute(&format!("__oryn_worker_terminate({id})"))
                .unwrap();
        }
    }

    #[test]
    fn worker_realms_install_generated_platform_bindings() {
        let mut host = RawV8Host::new();
        let source = serde_json::to_string(
            "try{const target=new EventTarget();const event=new Event('probe');let during=0;target.addEventListener('probe',e=>during=e.composedPath().length);target.dispatchEvent(event);class Request extends EventTarget{constructor(){super();this.onsuccess=null}finish(){this.dispatchEvent(new Event('success'))}}const request=new Request();let propertyHandler=false;request.onsuccess=()=>propertyHandler=true;request.finish();const domError=new DOMException('missing','NotFoundError');const blobUrl=URL.createObjectURL(new Blob(['x']));URL.revokeObjectURL(blobUrl);const controller=new AbortController(),combined=AbortSignal.any([controller.signal]);controller.abort();postMessage({blob:typeof Blob,event:event instanceof Event,target:event.target===target,during,after:event.composedPath().length,aborted:AbortSignal.abort().aborted,abortReason:AbortSignal.abort().reason instanceof DOMException,combined:combined.aborted&&combined.reason instanceof DOMException,propertyHandler,domException:domError instanceof DOMException,domName:domError.name,domCode:domError.code,url:new URL('/a','https://example.test/root').href,blobUrl:blobUrl.startsWith('blob:'),xhr:typeof XMLHttpRequest,fileReader:typeof FileReader})}catch(error){postMessage({error:String(error),stack:String(error&&error.stack)})}",
        )
        .unwrap();
        let id = host
            .evaluate_string(&format!(
                "__oryn_worker_create({source},'https://example.test/worker.js',false,'dedicatedworker')"
            ))
            .unwrap();
        let messages = host
            .evaluate_string(&format!("__oryn_worker_take({id})"))
            .unwrap();
        let messages: Vec<String> = serde_json::from_str(&messages).unwrap();
        let payload: serde_json::Value = serde_json::from_str(&messages[0]).unwrap();
        assert_eq!(payload["error"], serde_json::Value::Null, "{payload}");
        assert_eq!(payload["blob"], "function");
        assert_eq!(payload["event"], true);
        assert_eq!(payload["target"], true);
        assert_eq!(payload["during"], 1);
        assert_eq!(payload["after"], 0);
        assert_eq!(payload["aborted"], true);
        assert_eq!(payload["abortReason"], true);
        assert_eq!(payload["combined"], true);
        assert_eq!(payload["propertyHandler"], true);
        assert_eq!(payload["domException"], true);
        assert_eq!(payload["domName"], "NotFoundError");
        assert_eq!(payload["domCode"], 8);
        assert_eq!(payload["url"], "https://example.test/a");
        assert_eq!(payload["blobUrl"], true);
        assert_eq!(payload["xhr"], "function");
        assert_eq!(payload["fileReader"], "function");
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

    #[cfg(all(feature = "core-lite", not(feature = "core-full")))]
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
