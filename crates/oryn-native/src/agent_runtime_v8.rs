// Included in raw_v8 after host callback installation macros.
use crate::agent_thread::{AgentEndpoint, AgentRuntime};
use oryn_js::worker_wire::{
    AgentBootstrap, AgentCommand, AgentEvent, AgentKind, AgentServiceRequest, AgentServiceResponse,
    ScriptType, WorkerError, WorkerTransport,
};

struct AgentMessages(Arc<dyn WorkerTransport>);
struct AgentRoute {
    inner: Arc<dyn WorkerTransport>,
    stopping: Arc<AtomicBool>,
}
impl WorkerTransport for AgentRoute {
    fn retire(&self) {
        self.stopping.store(true, Ordering::Release);
        self.inner.retire();
    }
    fn drain(&self, timeout: Duration) -> Result<(), WorkerError> {
        self.inner.drain(timeout)
    }
    fn request(&self, request: AgentServiceRequest) -> Result<AgentServiceResponse, WorkerError> {
        if self.stopping.load(Ordering::Acquire)
            && !matches!(
                &request,
                AgentServiceRequest::PlatformRetire
                    | AgentServiceRequest::PlatformReleaseResult { .. }
                    | AgentServiceRequest::SharedMemory {
                        request: oryn_js::shared_memory::SharedMemoryRequest::CancelAdmission { .. }
                            | oryn_js::shared_memory::SharedMemoryRequest::Release { .. },
                        ..
                    }
            )
        {
            return Err(WorkerError::new("AbortError", "agent route retired"));
        }
        self.inner.request(request)
    }
}
impl oryn_js::message_wire::MessageTransport for AgentMessages {
    fn request(
        &self,
        request: oryn_js::message_wire::MessageRequest,
    ) -> Result<oryn_js::message_wire::MessageResponse, String> {
        match self
            .0
            .request(AgentServiceRequest::Message(request))
            .map_err(|error| error.to_string())?
        {
            AgentServiceResponse::Message(response) => Ok(response),
            _ => Err("agent message response mismatch".into()),
        }
    }
}
#[derive(Clone)]
struct AgentExecution {
    bootstrap: Arc<AgentBootstrap>,
    transport: Arc<dyn WorkerTransport>,
    endpoint: Option<AgentEndpoint>,
    closing: Arc<std::sync::atomic::AtomicBool>,
}
pub struct NativeAgentRuntime {
    service_startup_failure: Option<WorkerError>,
    worklet_failure: Option<WorkerError>,
    worklet_load: Option<u64>,
    host: RawV8Host,
    bootstrap: AgentBootstrap,
    evaluation: Option<v8::Global<v8::Promise>>,
    startup: Option<PendingAgentStartup>,
}
struct PendingAgentStartup {
    graph: ModuleGraphLoad,
    backend: ModuleGraphBackend,
    deadline: Instant,
    retry: Option<Instant>,
    prepared: bool,
    native_failure: Option<WorkerError>,
}
fn agent_error(error: impl std::fmt::Display) -> WorkerError {
    WorkerError::new("OperationError", error.to_string())
}
fn service_startup_error(scope: &mut v8::PinScope, message: v8::Local<v8::Message>) -> WorkerError {
    const MAX_BYTES: usize = 32 * 1024 - 1;
    const TRUNCATED: &str = " [truncated]";
    fn truncate_utf8(value: &mut String, limit: usize) -> bool {
        if value.len() <= limit {
            return false;
        }
        let mut end = limit;
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        value.truncate(end);
        true
    }
    let text = message.get(scope);
    let length = text.length();
    let mut units = vec![0; length.min(8192)];
    text.write_v2(scope, 0, &mut units, v8::WriteFlags::empty());
    let mut detail = String::from_utf16_lossy(&units);
    let mut truncated = length > units.len();
    // Native Message metadata never reads exception.name/message/stack or
    // invokes authored conversion. Bound the escaped resource in bytes too:
    // Debug escaping can expand control characters beyond their UTF-16 size.
    let mut resource = message
        .get_script_resource_name(scope)
        .and_then(|value| value.try_cast::<v8::String>().ok())
        .map(|text| {
            let mut units = vec![0; text.length().min(4096)];
            text.write_v2(scope, 0, &mut units, v8::WriteFlags::empty());
            truncated |= text.length() > units.len();
            format!("{:?}", String::from_utf16_lossy(&units))
        })
        .unwrap_or_else(|| "\"\"".into());
    truncated |= truncate_utf8(&mut resource, 4096);
    let line = message.get_line_number(scope).unwrap_or(0);
    let column = if line == 0 {
        0
    } else {
        message.get_start_column().saturating_add(1)
    };
    detail.push_str(&format!(
        " [worker startup source={resource} line={line} column={column} offset={}]",
        message.get_start_position()
    ));
    // Preserve the original bounded failure prefix and the terminal marker
    // expected by retained startup/reporting callers, including oversized input.
    truncated |= truncate_utf8(&mut detail, MAX_BYTES - TRUNCATED.len());
    if truncated {
        detail.push_str(TRUNCATED);
    }
    WorkerError::new("TypeError", detail)
}

impl NativeAgentRuntime {
    fn tasks_stopped(&self, endpoint: &AgentEndpoint) -> bool {
        endpoint.stopped() || self.host.worker_tasks_stopped()
    }
    /// Called on the agent's owning OS thread. No author script runs here.
    pub fn create(
        bootstrap: AgentBootstrap,
        transport: Arc<dyn WorkerTransport>,
    ) -> Result<Self, WorkerError> {
        let closing = Arc::new(AtomicBool::new(false));
        let transport: Arc<dyn WorkerTransport> = Arc::new(AgentRoute {
            inner: transport,
            stopping: Arc::new(AtomicBool::new(false)),
        });
        let url = Url::parse(&bootstrap.script_url).map_err(agent_error)?;
        let mut host = if bootstrap.origin.opaque {
            RawV8Host::new_with_heap_limit(bootstrap.limits.heap_bytes)
        } else {
            RawV8Host::new_with_base_url_and_heap_limit(url, bootstrap.limits.heap_bytes)
        };
        host.set_execution_wall_limit(Duration::from_millis(bootstrap.limits.task_wall_ms));
        host.isolate.set_slot(AgentExecution {
            bootstrap: Arc::new(bootstrap.clone()),
            transport: transport.clone(),
            endpoint: None,
            closing,
        });
        let realm = host.active_realm;
        {
            v8::scope!(let handles,&mut host.isolate);
            let old = v8::Local::new(handles, &host.contexts[&realm]);
            let environment = old
                .get_slot::<EnvironmentDescriptor>()
                .ok_or_else(|| agent_error("agent environment missing"))?;
            let descriptor = old
                .get_slot::<RealmDescriptor>()
                .ok_or_else(|| agent_error("agent realm missing"))?;
            // Workers use ordinary globals, never a WindowProxy or a creator's
            // context. Retain only native environment identity in the new realm.
            let context = v8::Context::new(handles, Default::default());
            context.set_slot(environment);
            context.set_slot(descriptor);
            context.set_slot(Rc::new(oryn_js::realm::SecureContextState(
                bootstrap.secure_context,
            )));
            context.set_slot(Rc::new(oryn_js::v8_bindings::WorkerExposureKind(
                bootstrap.kind,
            )));
            if bootstrap.kind != AgentKind::AudioWorklet {
                install_host_functions!(handles, context);
            }
            let scope = &mut v8::ContextScope::new(handles, context);
            let dom = oryn_dom::html::parse("<!doctype html>")
                .into_runtime()
                .map_err(agent_error)?;
            let root = dom.document();
            let state = install_generated_bindings_with_root_for_global(
                scope,
                context,
                Rc::new(RefCell::new(dom)),
                realm,
                1,
                root,
                if bootstrap.kind == AgentKind::AudioWorklet {
                    BindingGlobal::AudioWorklet
                } else {
                    BindingGlobal::Worker
                },
            )
            .map_err(agent_error)?;
            if bootstrap.kind == AgentKind::AudioWorklet {
                oryn_js::v8_bindings::audio_worklet::install(
                    scope,
                    bootstrap.options.audio_sample_rate.unwrap_or(48000.),
                )
                .map_err(agent_error)?;
            }
            state.set_native_timeout_scheduler(native_abort_timeout_schedule);
            state
                .commit_location(&bootstrap.script_url)
                .map_err(agent_error)?;
            state
                .set_document_referrer_policy(&bootstrap.referrer_policy)
                .map_err(agent_error)?;
            state.set_security_origin(bootstrap.origin.serialized.clone());
            state.set_message_transport(Arc::new(AgentMessages(transport.clone())));
            state.set_platform_host(Rc::new(crate::agent_platform::AgentPlatformHost::new(
                transport.clone(),
                bootstrap.origin.serialized.clone(),
            )));
            let trusted_transport = transport.clone();
            state.set_trusted_types_policy(Rc::new(move |_, check| {
                match trusted_transport.request(AgentServiceRequest::TrustedTypes(check.clone())) {
                    Ok(AgentServiceResponse::TrustedTypes { allowed }) => Ok(allowed),
                    Ok(_) => Err("invalid worker Trusted Types decision".into()),
                    Err(error) => Err(error.to_string()),
                }
            }));
            state.set_worker_transport(
                transport,
                Some(oryn_js::worker_wire::Creator::Agent(bootstrap.id)),
                Some(bootstrap.script_url.clone()),
            );
            let name = v8::String::new_from_two_byte(
                scope,
                &bootstrap.options.name,
                v8::NewStringType::Normal,
            )
            .ok_or_else(|| agent_error("worker name allocation"))?;
            let key = v8::String::new(scope, "name").unwrap();
            context.global(scope).define_own_property(
                scope,
                key.into(),
                name.into(),
                v8::PropertyAttribute::READ_ONLY,
            );
            // No PageId-based native context callback or legacy same-isolate
            // worker bridge is available to an independently owned agent.
            for name in [
                "__oryn_worker_create",
                "__oryn_worker_post",
                "__oryn_worker_take",
                "__oryn_worker_terminate",
                "__oryn_import_scripts",
            ] {
                let key = v8::String::new(scope, name)
                    .ok_or_else(|| agent_error("agent property allocation"))?;
                context.global(scope).delete(scope, key.into());
            }
            if bootstrap.kind != AgentKind::Service && bootstrap.kind != AgentKind::AudioWorklet {
                let function = v8::Function::new(scope, agent_close)
                    .ok_or_else(|| agent_error("agent close allocation"))?;
                let key = v8::String::new(scope, "close").unwrap();
                context
                    .global(scope)
                    .create_data_property(scope, key.into(), function.into());
            }
            if bootstrap.kind != AgentKind::AudioWorklet {
                let function = v8::Function::new(scope, agent_import_scripts)
                    .ok_or_else(|| agent_error("agent imports allocation"))?;
                let key = v8::String::new(scope, "importScripts").unwrap();
                context
                    .global(scope)
                    .create_data_property(scope, key.into(), function.into());
                oryn_js::v8_bindings::service_workers::install_worker_container(scope, context)
                    .map_err(agent_error)?;
                oryn_js::v8_bindings::cache_storage::install(scope, &state).map_err(agent_error)?;
            }
            if bootstrap.kind == AgentKind::Service {
                oryn_js::v8_bindings::service_workers::install_agent(scope, context)
                    .map_err(agent_error)?;
            }
            if bootstrap.kind == AgentKind::Dedicated {
                let function = v8::Function::new(scope, agent_post_message)
                    .ok_or_else(|| agent_error("agent postMessage allocation"))?;
                let key = v8::String::new(scope, "postMessage").unwrap();
                context
                    .global(scope)
                    .create_data_property(scope, key.into(), function.into());
            }
            let global = v8::Global::new(scope, context);
            let weak = v8::Weak::new(scope, context);
            scope
                .get_slot_mut::<NativeRealmIdentities>()
                .unwrap()
                .contexts
                .insert(realm, weak);
            host.contexts.insert(realm, global);
            host.binding_states.insert(realm, state);
        }
        if bootstrap.kind != AgentKind::AudioWorklet {
            // Independent agents bypass the Window runtime bootstrap. Install
            // once in the new worker realm, before author script evaluation.
            host.execute(crate::observable::OBSERVABLE_GLOBALS)
                .map_err(agent_error)?;
        }
        let policy_url = if bootstrap.policy.url.is_empty() {
            bootstrap.script_url.as_str()
        } else {
            bootstrap.policy.url.as_str()
        };
        let mut snapshot = bootstrap.policy.clone();
        if snapshot.url.is_empty() {
            snapshot.url = policy_url.into();
        }
        let policy = crate::csp::CspPolicySet::from_agent_policy(&snapshot);
        let trusted = policy.trusted_types();
        host.configure_trusted_types(
            oryn_js::trusted_types::TrustedTypesConfig {
                require_script: trusted.require_script,
                allowed_policy_names: trusted.policy_names,
                allow_any_policy: trusted.allow_any_policy,
                allow_duplicates: trusted.allow_duplicates,
                deny_policy_creation: trusted.deny_policy_creation,
            },
            policy.allows_eval(false).allowed,
        )
        .map_err(agent_error)?;
        Ok(Self {
            service_startup_failure: None,
            worklet_failure: None,
            worklet_load: None,
            host,
            bootstrap,
            evaluation: None,
            startup: None,
        })
    }
    fn start(&mut self) -> Result<(), WorkerError> {
        self.worklet_failure = None;
        let url = serde_json::to_string(&self.bootstrap.script_url).map_err(agent_error)?;
        let origin =
            serde_json::to_string(&self.bootstrap.origin.serialized).map_err(agent_error)?;
        let scope_name = match self.bootstrap.kind {
            AgentKind::Dedicated => "DedicatedWorkerGlobalScope",
            AgentKind::Shared => "SharedWorkerGlobalScope",
            AgentKind::Service => "ServiceWorkerGlobalScope",
            AgentKind::AudioWorklet => "AudioWorkletGlobalScope",
        };
        // Bootstrap only captures installed native callbacks. It does not
        // replace native EventTarget, MessageEvent, or structuredClone.
        if self.bootstrap.kind != AgentKind::AudioWorklet {
            self.host.execute(&format!(r#"(()=>{{
            const schedule=globalThis.__oryn_timer_schedule,cancel=globalThis.__oryn_timer_cancel;
            class WorkerGlobalScope extends EventTarget {{constructor(){{throw new TypeError('Illegal constructor')}}}}
            class DedicatedWorkerGlobalScope extends WorkerGlobalScope {{}}
            class SharedWorkerGlobalScope extends WorkerGlobalScope {{}}
            class ServiceWorkerGlobalScope extends WorkerGlobalScope {{}}
            for(const type of [WorkerGlobalScope,DedicatedWorkerGlobalScope,SharedWorkerGlobalScope,ServiceWorkerGlobalScope]){{
                Object.defineProperty(type.prototype,Symbol.toStringTag,{{value:type.name,configurable:true}});
                if(type===WorkerGlobalScope||type.name==='{scope_name}')globalThis[type.name]=type;
            }}
            Object.setPrototypeOf(globalThis,globalThis.{scope_name}.prototype);
            globalThis.self=globalThis;
            const address=new URL({url}),parts={{}};
            for(const key of ['href','protocol','host','hostname','port','pathname','search','hash'])parts[key]=address[key];parts.origin={origin};
            class WorkerLocation {{constructor(){{throw new TypeError('Illegal constructor')}}}}
            const location=Object.create(WorkerLocation.prototype);
            for(const key of Object.keys(parts))Object.defineProperty(WorkerLocation.prototype,key,{{get(){{if(this!==location)throw new TypeError('WorkerLocation receiver');return parts[key]}},enumerable:true,configurable:true}});
            Object.defineProperty(WorkerLocation.prototype,'toString',{{value:function toString(){{if(this!==location)throw new TypeError('WorkerLocation receiver');return parts.href}},writable:true,configurable:true}});
            Object.defineProperty(WorkerLocation.prototype,Symbol.toStringTag,{{value:'WorkerLocation',configurable:true}});
            globalThis.WorkerLocation=WorkerLocation;Object.defineProperty(globalThis,'location',{{value:location,writable:false,enumerable:true,configurable:false}});
            globalThis.setTimeout=(fn,delay=0,...args)=>schedule(fn,delay,false,args);
            globalThis.setInterval=(fn,delay=0,...args)=>schedule(fn,delay,true,args);
            globalThis.clearTimeout=globalThis.clearInterval=id=>cancel(Number(id));
        }})()"#)).map_err(agent_error)?;
        }
        if self.bootstrap.options.script_type == ScriptType::Module {
            let source = String::from_utf16_lossy(&self.bootstrap.source);
            let name = &self.bootstrap.script_url;
            self.host
                .retain_verified_module_fetch(name, name, &source)
                .map_err(agent_error)?;
            let include = self.bootstrap.options.credentials
                == oryn_js::worker_wire::WorkerCredentials::Include;
            self.host.set_module_credentials(name, include);
            self.host
                .set_module_referrer_policy(name, &self.bootstrap.referrer_policy, false);
            let registry = self
                .host
                .isolate
                .get_slot::<Rc<RefCell<DynamicImports>>>()
                .expect("worker import scheduler")
                .clone();
            let (bytes, slots, fetches) = {
                let registry = registry.borrow();
                (
                    registry.bytes.clone(),
                    registry.slots.clone(),
                    registry.fetches.clone(),
                )
            };
            let mut graph = ModuleGraphLoad::with_root_request(
                name,
                &source,
                Some(name),
                bytes,
                slots,
                fetches,
            )
            .map_err(agent_error)?;
            graph.retain_document_authority();
            let backend = self.host.module_fetch_backend().map_err(agent_error)?;
            self.startup = Some(PendingAgentStartup {
                graph,
                backend,
                deadline: Instant::now() + Duration::from_secs(60),
                retry: None,
                prepared: false,
                native_failure: None,
            });
            self.resume_startup()
        } else {
            self.evaluate_startup(None)
        }
    }
    fn resume_startup(&mut self) -> Result<(), WorkerError> {
        let Some(mut pending) = self.startup.take() else {
            return Ok(());
        };
        if Instant::now() >= pending.deadline {
            return Err(pending.native_failure.take().unwrap_or_else(|| {
                WorkerError::new("TimeoutError", "worker module graph deadline exceeded")
            }));
        }
        if pending.retry.is_some_and(|retry| Instant::now() < retry) {
            self.startup = Some(pending);
            return Ok(());
        }
        if !pending.prepared {
            match pending.graph.poll_with_backend(
                &mut self.host,
                &pending.backend,
                &mut BTreeMap::new(),
            ) {
                Ok(false) => {
                    self.startup = Some(pending);
                    return Ok(());
                }
                Ok(true) => pending.prepared = true,
                Err(HostError::ScriptLoad(
                    failure @ crate::script_load_failure::ScriptLoadFailure::Infrastructure(_),
                )) => {
                    if pending.native_failure.is_none() {
                        pending.native_failure = Some(failure.into_worker());
                    }
                    pending.retry = Some(Instant::now() + Duration::from_millis(10));
                    self.startup = Some(pending);
                    return Ok(());
                }
                Err(error) => return Err(agent_error(error)),
            }
        }
        match self.evaluate_startup(pending.graph.error_token) {
            Err(error)
                if error.script_failure
                    == Some(oryn_js::worker_wire::ScriptFailureKind::Infrastructure) =>
            {
                if pending.native_failure.is_none() {
                    pending.native_failure = Some(error);
                }
                pending.retry = Some(Instant::now() + Duration::from_millis(10));
                self.startup = Some(pending);
                Ok(())
            }
            result => result,
        }
    }

    fn evaluate_startup(&mut self, graph_error: Option<u64>) -> Result<(), WorkerError> {
        let deadline = ExecutionDeadline::arm(&self.host.isolate, self.host.execution_wall_limit);
        v8::scope!(let handles,&mut self.host.isolate);
        let context = v8::Local::new(handles, &self.host.contexts[&self.host.active_realm]);
        let scope = &mut v8::ContextScope::new(handles, context);
        let _ = take_module_preparation_failure(scope);
        v8::tc_scope!(let scope,scope);
        if self.bootstrap.options.script_type == ScriptType::Module {
            context.set_slot(Rc::new(PreparedImportLink {
                root: RefCell::new(None),
                error: graph_error,
            }));
        }
        let result = if self.bootstrap.options.script_type == ScriptType::Module {
            if throw_module_graph_error(scope, graph_error) {
                None
            } else {
                agent_compile_module(
                    scope,
                    &self.bootstrap.script_url,
                    &self.bootstrap.script_url,
                    &self.bootstrap.source,
                )
                .and_then(|module| {
                    if module.instantiate_module(scope, agent_resolve_module) != Some(true) {
                        return None;
                    }
                    let promise = evaluate_module_once(scope, module)?;
                    scope.perform_microtask_checkpoint();
                    if promise.state() == v8::PromiseState::Rejected {
                        if self.bootstrap.kind == AgentKind::AudioWorklet {
                            self.worklet_failure = Some(WorkerError::new(
                                "AbortError",
                                "worklet module evaluation failed",
                            ));
                        }
                        let exception = promise.result(scope);
                        let message = v8::Exception::create_message(scope, exception);
                        if self.bootstrap.kind == AgentKind::Service {
                            self.service_startup_failure =
                                Some(service_startup_error(scope, message));
                        }
                        let reported =
                            oryn_js::v8_bindings::report_exception(scope, exception, Some(message));
                        if self.bootstrap.kind != AgentKind::Service {
                            reported.ok()?;
                        }
                        return Some(());
                    }
                    self.evaluation = Some(v8::Global::new(scope, promise));
                    Some(())
                })
            }
        } else {
            agent_classic(
                scope,
                &self.bootstrap.script_url,
                &self.bootstrap.source,
                Some(&self.bootstrap.referrer_policy),
            )
        };
        context.remove_slot::<PreparedImportLink>();
        if deadline.expired() || scope.is_execution_terminating() {
            return Err(WorkerError::new(
                "TimeoutError",
                "worker startup interrupted",
            ));
        }
        if let Some(failure) = take_module_preparation_failure(scope) {
            if scope.has_caught() {
                scope.reset();
            }
            return Err(failure.into_worker());
        }
        if result.is_none() {
            if self.bootstrap.kind == AgentKind::AudioWorklet {
                self.worklet_failure = Some(WorkerError::new(
                    "AbortError",
                    "worklet module compilation/evaluation failed",
                ));
            }
            let exception = scope.exception();
            let message = scope.message();
            scope.reset();
            let Some(exception) = exception else {
                return Err(agent_error(
                    "worker source evaluation failed without an exception",
                ));
            };
            if self.bootstrap.kind == AgentKind::Service {
                let diagnostic =
                    message.unwrap_or_else(|| v8::Exception::create_message(scope, exception));
                self.service_startup_failure = Some(service_startup_error(scope, diagnostic));
            }
            let reported = oryn_js::v8_bindings::report_exception(scope, exception, message);
            if self.bootstrap.kind != AgentKind::Service {
                reported.map_err(agent_error)?;
            }
        }
        scope.perform_microtask_checkpoint();
        if deadline.expired() || scope.is_execution_terminating() {
            return Err(WorkerError::new(
                "TimeoutError",
                "worker startup interrupted",
            ));
        }
        Ok(())
    }
    fn dispatch(&mut self, command: AgentCommand) -> Result<(), WorkerError> {
        if let AgentCommand::Worklet(oryn_js::worker_wire::WorkletCommand::LoadModule {
            id,
            url,
            credentials,
        }) = &command
        {
            if self.bootstrap.kind != AgentKind::AudioWorklet {
                return Err(agent_error("module addressed to non-worklet"));
            }
            let transport = self
                .host
                .isolate
                .get_slot::<AgentExecution>()
                .unwrap()
                .transport
                .clone();
            let response = transport.request(AgentServiceRequest::ImportWorkletModule {
                url: url.clone(),
                credentials: *credentials,
                referrer_source: self.bootstrap.script_url.clone(),
                referrer_policy: self.bootstrap.referrer_policy.clone(),
            });
            match response {
                Ok(AgentServiceResponse::Script {
                    url,
                    source,
                    referrer_policy,
                }) => {
                    self.bootstrap.script_url = url;
                    self.bootstrap.source = source;
                    self.bootstrap.referrer_policy = referrer_policy;
                    self.bootstrap.options.credentials = *credentials;
                    self.worklet_load = Some(*id);
                    self.evaluation = None;
                    if let Err(error) = self.start() {
                        self.worklet_failure = Some(error);
                        self.startup = None;
                        self.evaluation = None;
                    }
                    return Ok(());
                }
                other => {
                    let result = Err(match other {
                        Err(error) => error,
                        _ => agent_error("invalid worklet module response"),
                    });
                    if let Some(endpoint) = self
                        .host
                        .isolate
                        .get_slot::<AgentExecution>()
                        .and_then(|agent| agent.endpoint.clone())
                    {
                        endpoint.emit(AgentEvent::WorkletFinished { id: *id, result })?;
                    }
                    return Ok(());
                }
            }
        }

        let deadline = ExecutionDeadline::arm(&self.host.isolate, self.host.execution_wall_limit);
        v8::scope!(let handles,&mut self.host.isolate);
        let context = v8::Local::new(handles, &self.host.contexts[&self.host.active_realm]);
        let scope = &mut v8::ContextScope::new(handles, context);
        let state = context
            .get_slot::<V8BindingState>()
            .ok_or_else(|| agent_error("retired worker"))?;
        match command {
            AgentCommand::Message { value, .. } => {
                oryn_js::v8_bindings::worker_events::message(scope, &state, value)
                    .map_err(agent_error)?
            }
            AgentCommand::Connect { port, .. } => {
                if self.bootstrap.kind != AgentKind::Shared {
                    return Err(agent_error("connect addressed to non-shared worker"));
                }
                oryn_js::v8_bindings::worker_events::connect(scope, &state, port)
                    .map_err(agent_error)?;
            }
            AgentCommand::Worklet(oryn_js::worker_wire::WorkletCommand::Construct {
                id,
                name,
                value,
            }) => {
                if self.bootstrap.kind != AgentKind::AudioWorklet {
                    return Err(agent_error("processor addressed to non-worklet"));
                }
                let result = {
                    v8::tc_scope!(let tc,scope);
                    let result = oryn_js::v8_bindings::audio_worklet::construct(
                        tc, &state, id, &name, *value,
                    )
                    .map_err(agent_error);
                    if tc.has_caught() {
                        tc.reset();
                    }
                    result
                };
                if let Some(endpoint) = scope
                    .get_slot::<AgentExecution>()
                    .and_then(|agent| agent.endpoint.clone())
                {
                    endpoint.emit(AgentEvent::WorkletFinished { id, result })?;
                }
            }
            AgentCommand::Worklet(oryn_js::worker_wire::WorkletCommand::LoadModule { .. }) => {
                unreachable!()
            }
            AgentCommand::Stop => {}
            AgentCommand::FetchPull { id } => {
                if self.bootstrap.kind != AgentKind::Service {
                    return Err(agent_error(
                        "service body demand addressed to non-service worker",
                    ));
                }
                oryn_js::v8_bindings::service_worker_events::pull(&state, id)
                    .map_err(agent_error)?;
            }
            AgentCommand::CancelTask { id } => {
                if self.bootstrap.kind != AgentKind::Service {
                    return Err(agent_error(
                        "service cancellation addressed to non-service worker",
                    ));
                }
                let output = oryn_js::v8_bindings::service_worker_events::cancel(scope, &state, id)
                    .map_err(agent_error)?;
                if let Some(endpoint) = scope
                    .get_slot::<AgentExecution>()
                    .and_then(|agent| agent.endpoint.clone())
                {
                    for event in output {
                        endpoint.emit(event)?;
                    }
                }
            }
            command @ (AgentCommand::Lifecycle { .. }
            | AgentCommand::Fetch(_)
            | AgentCommand::ServiceMessage { .. }) => {
                if self.bootstrap.kind != AgentKind::Service {
                    return Err(agent_error("service event addressed to non-service worker"));
                }
                oryn_js::v8_bindings::service_worker_events::begin(scope, &state, command)
                    .map_err(agent_error)?;
            }
        }
        scope.perform_microtask_checkpoint();
        if deadline.expired() || scope.is_execution_terminating() {
            return Err(WorkerError::new("TimeoutError", "worker task interrupted"));
        }
        Ok(())
    }
}
impl AgentRuntime for NativeAgentRuntime {
    fn cleanup(&self) -> Arc<dyn Fn(Duration) -> Result<(), WorkerError> + Send + Sync> {
        let transport = self
            .host
            .isolate
            .get_slot::<AgentExecution>()
            .unwrap()
            .transport
            .clone();
        Arc::new(move |timeout| {
            let start = std::time::Instant::now();
            let waits = oryn_js::shared_wait_custody::drain(&transport, timeout)
                .map_err(|error| WorkerError::new("InvalidStateError", error));
            let backend = transport.drain(timeout.saturating_sub(start.elapsed()));
            waits.and(backend)
        })
    }
    fn interrupt(&self) -> Arc<dyn Fn() + Send + Sync> {
        let handle = self.host.isolate.thread_safe_handle();
        let stopping = self
            .host
            .isolate
            .get_slot::<AgentExecution>()
            .unwrap()
            .closing
            .clone();
        let transport = self
            .host
            .isolate
            .get_slot::<AgentExecution>()
            .unwrap()
            .transport
            .clone();
        Arc::new(move || {
            stopping.store(true, Ordering::Release);
            handle.terminate_execution();
            transport.retire();
        })
    }
    fn run(mut self, endpoint: AgentEndpoint) -> Result<(), WorkerError> {
        self.host
            .isolate
            .get_slot_mut::<AgentExecution>()
            .unwrap()
            .endpoint = Some(endpoint.clone());
        for state in self.host.binding_states.values() {
            let endpoint = endpoint.clone();
            state.set_worker_error_reporter(Rc::new(move |event| {
                endpoint.emit(event).map_err(|error| error.to_string())
            }));
        }
        let result = (|| {
            let Some(elapsed) = endpoint.suspension_checkpoint_with_elapsed()? else {
                return Ok(());
            };
            self.host
                .exclude_suspended_timer_time(elapsed)
                .map_err(agent_error)?;
            if let Err(error) = self.start() {
                if self.bootstrap.kind != AgentKind::AudioWorklet {
                    return Err(error);
                }
                self.startup = None;
                self.evaluation = None;
                self.worklet_failure = Some(error.clone());
                endpoint.emit(AgentEvent::Error {
                    message: error.message.encode_utf16().collect(),
                    filename: self.bootstrap.script_url.clone(),
                    line: 0,
                    column: 0,
                })?;
            }
            let mut ready = false;
            let mut startup_report_deadline = None;
            loop {
                if self.tasks_stopped(&endpoint) {
                    break;
                }
                let Some(elapsed) = endpoint.suspension_checkpoint_with_elapsed()? else {
                    break;
                };
                self.host
                    .exclude_suspended_timer_time(elapsed)
                    .map_err(agent_error)?;
                if endpoint.stopped()
                    || self
                        .host
                        .isolate
                        .get_slot::<AgentExecution>()
                        .unwrap()
                        .closing
                        .load(std::sync::atomic::Ordering::Acquire)
                {
                    break;
                }
                if !ready {
                    if let Err(error) = self.resume_startup() {
                        if self.bootstrap.kind != AgentKind::AudioWorklet {
                            return Err(error);
                        }
                        self.startup = None;
                        self.evaluation = None;
                        self.worklet_failure = Some(error.clone());
                        if self.worklet_load.is_none() {
                            endpoint.emit(AgentEvent::Error {
                                message: error.message.encode_utf16().collect(),
                                filename: self.bootstrap.script_url.clone(),
                                line: 0,
                                column: 0,
                            })?;
                        }
                    }
                    if self.tasks_stopped(&endpoint) {
                        break;
                    }
                    let settled = self.startup.is_none() && {
                        v8::scope!(let scope,&mut self.host.isolate);
                        match &self.evaluation {
                            None => true,
                            Some(promise) => match v8::Local::new(scope, promise).state() {
                                v8::PromiseState::Pending => false,
                                v8::PromiseState::Fulfilled => true,
                                v8::PromiseState::Rejected => {
                                    if self.bootstrap.kind == AgentKind::AudioWorklet {
                                        self.worklet_failure = Some(WorkerError::new(
                                            "AbortError",
                                            "worklet module evaluation rejected",
                                        ));
                                    }
                                    let context = v8::Local::new(
                                        scope,
                                        &self.host.contexts[&self.host.active_realm],
                                    );
                                    let scope = &mut v8::ContextScope::new(scope, context);
                                    let promise = v8::Local::new(scope, promise);
                                    let exception = promise.result(scope);
                                    if self.bootstrap.kind == AgentKind::Service {
                                        let message =
                                            v8::Exception::create_message(scope, exception);
                                        self.service_startup_failure =
                                            Some(service_startup_error(scope, message));
                                    }
                                    let reported = oryn_js::v8_bindings::report_exception(
                                        scope, exception, None,
                                    );
                                    if self.bootstrap.kind != AgentKind::Service {
                                        reported.map_err(agent_error)?;
                                    }
                                    true
                                }
                            },
                        }
                    };
                    if self.tasks_stopped(&endpoint) {
                        break;
                    }
                    if settled {
                        self.evaluation = None;
                    }
                    // Actual evaluation failure is independent of ErrorEvent
                    // cancellation. Retained report publication gets its own
                    // retry boundary before teardown; unrelated task errors do
                    // not decide whether module startup succeeded.
                    let startup_reports_pending = self.bootstrap.kind == AgentKind::Service
                        && self
                            .host
                            .binding_states
                            .values()
                            .any(|state| state.has_pending_script_error_reports());
                    if self.service_startup_failure.is_some() && startup_reports_pending {
                        let report_deadline = startup_report_deadline.get_or_insert_with(|| {
                            Instant::now()
                                + self.host.execution_wall_limit.min(Duration::from_secs(5))
                        });
                        if Instant::now() >= *report_deadline {
                            return Err(self.service_startup_failure.take().unwrap());
                        }
                        {
                            let remaining =
                                report_deadline.saturating_duration_since(Instant::now());
                            let deadline = ExecutionDeadline::arm(
                                &self.host.isolate,
                                self.host.execution_wall_limit.min(remaining),
                            );
                            v8::scope!(let handles,&mut self.host.isolate);
                            let context = v8::Local::new(
                                handles,
                                &self.host.contexts[&self.host.active_realm],
                            );
                            let scope = &mut v8::ContextScope::new(handles, context);
                            let state = context
                                .get_slot::<V8BindingState>()
                                .ok_or_else(|| agent_error("retired worker"))?;
                            // The report owns its exception and listener progress on
                            // a transient failure, and supplies its own retry delay.
                            let _ = state.poll_script_error_reports(scope);
                            if deadline.expired() || scope.is_execution_terminating() {
                                return Err(self.service_startup_failure.take().unwrap());
                            }
                        }
                        std::thread::park_timeout(Duration::from_millis(1));
                        continue;
                    }
                    if settled && !startup_reports_pending {
                        if let Some(failure) = self.service_startup_failure.take() {
                            return Err(failure);
                        }
                        if self.bootstrap.kind == AgentKind::AudioWorklet {
                            v8::scope!(let scope,&mut self.host.isolate);
                            let context =
                                v8::Local::new(scope, &self.host.contexts[&self.host.active_realm]);
                            let scope = &mut v8::ContextScope::new(scope, context);
                            endpoint.emit(AgentEvent::WorkletDefinitions(
                                oryn_js::v8_bindings::audio_worklet::definitions(scope),
                            ))?;
                        }
                        if let Some(id) = self.worklet_load.take() {
                            endpoint.emit(AgentEvent::WorkletFinished {
                                id,
                                result: self.worklet_failure.take().map_or(Ok(()), Err),
                            })?;
                        } else {
                            endpoint.emit(AgentEvent::Ready)?;
                        }
                        ready = true;
                    }
                }
                if ready {
                    if let Some(command) = endpoint.receive(Duration::from_millis(1))? {
                        if matches!(command, AgentCommand::Stop) {
                            break;
                        }
                        self.dispatch(command)?;
                        if self.worklet_load.is_some() {
                            ready = false;
                        }
                    }
                } else {
                    std::thread::park_timeout(Duration::from_millis(1));
                }
                if self.tasks_stopped(&endpoint) {
                    break;
                }
                self.host
                    .drain_scheduled_timers(false)
                    .map_err(agent_error)?;
                if self.tasks_stopped(&endpoint) {
                    break;
                }
                self.host.drain_async_fetches(false).map_err(agent_error)?;
                if self.tasks_stopped(&endpoint) {
                    break;
                }
                {
                    let deadline =
                        ExecutionDeadline::arm(&self.host.isolate, self.host.execution_wall_limit);
                    v8::scope!(let handles,&mut self.host.isolate);
                    let context =
                        v8::Local::new(handles, &self.host.contexts[&self.host.active_realm]);
                    let scope = &mut v8::ContextScope::new(handles, context);
                    let state = context
                        .get_slot::<V8BindingState>()
                        .ok_or_else(|| agent_error("retired worker"))?;
                    state.poll_external_ports(scope).map_err(agent_error)?;
                    if !state.execution_tasks_stopped() && self.bootstrap.kind == AgentKind::Service
                    {
                        for event in
                            oryn_js::v8_bindings::service_worker_events::poll(scope, &state)
                                .map_err(agent_error)?
                        {
                            endpoint.emit(event)?;
                        }
                    }
                    if deadline.expired() || scope.is_execution_terminating() {
                        return Err(WorkerError::new(
                            "TimeoutError",
                            "worker external task interrupted",
                        ));
                    }
                }
            }
            Ok(())
        })();
        self.startup = None;
        self.host
            .isolate
            .get_slot::<AgentExecution>()
            .unwrap()
            .closing
            .store(true, Ordering::Release);
        for state in self.host.binding_states.values() {
            state.retire_execution();
        }
        self.host
            .isolate
            .get_slot::<AgentExecution>()
            .unwrap()
            .transport
            .retire();
        result
    }
}
fn agent_close(scope: &mut v8::PinScope, _: v8::FunctionCallbackArguments, _: v8::ReturnValue) {
    if let Some(agent) = scope.get_slot::<AgentExecution>() {
        agent
            .closing
            .store(true, std::sync::atomic::Ordering::Release);
    }
    if let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() {
        state.close_worker_tasks();
    }
}
fn agent_post_message(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _: v8::ReturnValue,
) {
    let Some(agent) = scope.get_slot::<AgentExecution>().cloned() else {
        return;
    };
    if args.length() == 0 {
        throw_type_error(scope, "postMessage requires a message");
        return;
    }
    let Some(options) =
        oryn_js::v8_bindings::structured_clone::post_transfer_options(scope, args.get(1))
    else {
        return;
    };
    let transfers = options.transfers;
    let Some(value) = oryn_js::v8_bindings::structured_clone::serialize_wire_with_activation(
        scope,
        args.get(0),
        &transfers,
        |scope| {
            oryn_js::v8_bindings::structured_clone::activation_snapshot(
                scope,
                options.include_user_activation,
            )
        },
    ) else {
        return;
    };
    let mut shared_send_guard =
        oryn_js::v8_bindings::structured_clone::shared_send_guard(scope, &value);
    let ports = value.ports.clone();
    if agent.endpoint.as_ref().is_none_or(|endpoint| {
        endpoint
            .emit(AgentEvent::Message {
                client: None,
                value,
            })
            .is_err()
    }) {
        let _ = agent.transport.request(AgentServiceRequest::Message(
            oryn_js::message_wire::MessageRequest::DiscardTransfers { ports },
        ));
        oryn_js::v8_bindings::window_messages::throw_external_message_error(
            scope,
            "QuotaExceededError",
        );
    } else {
        shared_send_guard.transfer_custody();
    }
}
fn agent_classic(
    scope: &mut v8::PinScope,
    name: &str,
    source: &[u16],
    policy: Option<&str>,
) -> Option<()> {
    let text = v8::String::new_from_two_byte(scope, source, v8::NewStringType::Normal)?;
    let agent = scope.get_slot::<AgentExecution>()?.clone();
    let policy = policy.unwrap_or(&agent.bootstrap.referrer_policy);
    let options = oryn_js::v8_bindings::script_import_options(
        scope,
        agent.bootstrap.options.credentials == oryn_js::worker_wire::WorkerCredentials::Include,
        policy,
        name,
        false,
    )
    .ok()?;
    let name = v8::String::new(scope, name)?;
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
        Some(options.into()),
    );
    v8::Script::compile(scope, text, Some(&origin))?.run(scope)?;
    Some(())
}
fn agent_import_scripts(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    _: v8::ReturnValue,
) {
    let Some(agent) = scope.get_slot::<AgentExecution>().cloned() else {
        return;
    };
    if agent.bootstrap.options.script_type == ScriptType::Module {
        throw_type_error(scope, "importScripts is unavailable in module workers");
        return;
    }
    let mut urls = Vec::new();
    for index in 0..args.length() {
        let Some(value) = args.get(index).to_string(scope) else {
            return;
        };
        let value = value.to_rust_string_lossy(scope);
        let Ok(url) = Url::parse(&agent.bootstrap.script_url).and_then(|base| base.join(&value))
        else {
            oryn_js::v8_bindings::window_messages::throw_external_message_error(
                scope,
                "SyntaxError",
            );
            return;
        };
        urls.push(url.to_string());
    }
    for url in urls {
        match agent
            .transport
            .request(AgentServiceRequest::ImportScript { url, module: false })
        {
            Ok(AgentServiceResponse::Script {
                url,
                source,
                referrer_policy,
            }) => {
                if agent_classic(scope, &url, &source, Some(&referrer_policy)).is_none() {
                    return;
                }
            }
            _ => {
                oryn_js::v8_bindings::window_messages::throw_external_message_error(
                    scope,
                    "NetworkError",
                );
                return;
            }
        }
    }
}
fn agent_compile_module<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    requested: &str,
    url: &str,
    text: &[u16],
) -> Option<v8::Local<'s, v8::Module>> {
    let environment = current_environment(scope).ok()?;
    let key = (environment.id, requested.to_string());
    if let Some(module) = scope
        .get_slot::<ModuleCache>()
        .and_then(|cache| cache.0.get(&key))
        .cloned()
    {
        return Some(v8::Local::new(scope, &module));
    }
    let text = v8::String::new_from_two_byte(scope, text, v8::NewStringType::Normal)?;
    let name = v8::String::new(scope, url)?;
    let agent = scope.get_slot::<AgentExecution>()?.clone();
    let include =
        agent.bootstrap.options.credentials == oryn_js::worker_wire::WorkerCredentials::Include;
    let policy = module_referrer_policies(scope)
        .borrow()
        .0
        .get(requested)
        .cloned()
        .unwrap_or_else(|| agent.bootstrap.referrer_policy.clone());
    let options =
        oryn_js::v8_bindings::script_import_options(scope, include, &policy, url, false).ok()?;
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
        Some(options.into()),
    );
    let mut source = v8::script_compiler::Source::new(text, Some(&origin));
    let module = v8::script_compiler::compile_module(scope, &mut source)?;
    let identity = v8::Global::new(scope, module);
    scope
        .get_slot_mut::<ModuleCredentialRecords>()?
        .0
        .insert(identity.clone(), (include, policy));
    // Response URL supplies import.meta and relative resolution; requested URL
    // supplies module identity and the exact retirement cache key.
    scope
        .get_slot_mut::<ModuleOwners>()?
        .0
        .insert(identity.clone(), environment.id);
    let global = v8::Global::new(scope, module);
    scope
        .get_slot_mut::<ModuleNames>()?
        .0
        .insert(identity, url.into());
    scope.get_slot_mut::<ModuleCache>()?.0.insert(key, global);
    Some(module)
}
fn agent_load_javascript_module<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    base: &str,
    specifier: &str,
    policy: &str,
) -> Option<v8::Local<'s, v8::Module>> {
    let environment = current_environment(scope).ok()?;
    let agent = scope.get_slot::<AgentExecution>()?.clone();
    let captured = prepared_import_root(scope);
    let Some(url) = captured.or_else(|| agent_module_url(base, specifier)) else {
        throw_type_error(
            scope,
            "worker module specifier must be a URL or relative path",
        );
        return None;
    };
    let key = (environment.id, url.clone());
    if let Some(module) = scope
        .get_slot::<ModuleCache>()
        .and_then(|cache| cache.0.get(&key))
        .cloned()
    {
        return Some(v8::Local::new(scope, module));
    }
    let results = module_fetch_results(scope);
    let cached = results.borrow().0.get(&url).cloned();
    if let Some(cached) = cached {
        let Some((final_url, source)) = cached else {
            throw_type_error(scope, "worker module fetch previously failed");
            return None;
        };
        return agent_compile_module(
            scope,
            &url,
            &final_url,
            &source.encode_utf16().collect::<Vec<_>>(),
        );
    }
    if scope
        .get_current_context()
        .get_slot::<PreparedImportLink>()
        .is_some()
    {
        throw_type_error(scope, "prepared worker module response missing");
        return None;
    }
    let _ = (environment, agent, policy);
    throw_type_error(scope, "worker module source was not prepared");
    None
}
fn agent_resolve_module<'s>(
    context: v8::Local<'s, v8::Context>,
    specifier: v8::Local<'s, v8::String>,
    attributes: v8::Local<'s, v8::FixedArray>,
    referrer: v8::Local<'s, v8::Module>,
) -> Option<v8::Local<'s, v8::Module>> {
    v8::callback_scope!(unsafe scope,context);
    let identity = v8::Global::new(scope, referrer);
    let environment = current_environment(scope).ok()?;
    if scope.get_slot::<ModuleOwners>()?.0.get(&identity) != Some(&environment.id) {
        return None;
    }
    let base = scope.get_slot::<ModuleNames>()?.0.get(&identity)?.clone();
    let specifier = specifier.to_rust_string_lossy(scope);
    let policy = scope
        .get_slot::<ModuleCredentialRecords>()
        .and_then(|records| records.0.get(&identity))
        .map(|(_, policy)| policy.clone())
        .unwrap_or_else(|| module_referrer_policy(scope, &base));
    let kind = match import_module_type(scope, attributes, true) {
        Ok(kind) => kind,
        Err(error) => {
            throw_type_error(scope, &error);
            return None;
        }
    };
    match kind {
        crate::ModuleType::JavaScript => {
            agent_load_javascript_module(scope, &base, &specifier, &policy)
        }
        crate::ModuleType::Json | crate::ModuleType::Text => {
            load_typed_module_record(scope, &base, &specifier, kind, false, &policy)
        }
        crate::ModuleType::Css => {
            throw_type_error(scope, "CSS modules are unavailable in worker globals");
            None
        }
    }
}
fn agent_module_url(base: &str, specifier: &str) -> Option<String> {
    if !specifier.starts_with('/')
        && !specifier.starts_with("./")
        && !specifier.starts_with("../")
        && Url::parse(specifier).is_err()
    {
        return None;
    }
    Some(Url::parse(base).ok()?.join(specifier).ok()?.to_string())
}
fn agent_dynamic_import<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: v8::Local<'s, v8::Data>,
    resource: v8::Local<'s, v8::Value>,
    specifier: v8::Local<'s, v8::String>,
    phase: v8::ModuleImportPhase,
    attributes: v8::Local<'s, v8::FixedArray>,
) -> Option<PreparedModuleImport> {
    let _ = take_module_preparation_failure(scope);
    v8::tc_scope!(let scope,scope);
    let loaded = (|| {
        let agent = scope.get_slot::<AgentExecution>()?.clone();
        if agent.bootstrap.kind == AgentKind::Service {
            throw_type_error(scope, "dynamic import is unavailable in service workers");
            return None;
        }
        let options = v8::Local::<v8::FixedArray>::try_from(options)
            .ok()
            .filter(|options| options.length() == 4);
        let policy = options
            .and_then(|options| options.get(scope, 1))
            .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
            .map(|value| value.to_rust_string_lossy(scope))
            .unwrap_or_else(|| agent.bootstrap.referrer_policy.clone());
        let base = options
            .and_then(|options| options.get(scope, 2))
            .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
            .map(|value| value.to_rust_string_lossy(scope))
            .unwrap_or_else(|| resource.to_rust_string_lossy(scope));
        let specifier = specifier.to_rust_string_lossy(scope);
        let base = if Url::parse(&base).is_ok() {
            base
        } else {
            agent.bootstrap.script_url.clone()
        };
        let kind = match import_module_type(scope, attributes, false) {
            Ok(kind) => kind,
            Err(error) => {
                throw_type_error(scope, &error);
                return None;
            }
        };
        let module = match kind {
            crate::ModuleType::JavaScript => {
                agent_load_javascript_module(scope, &base, &specifier, &policy)?
            }
            crate::ModuleType::Json | crate::ModuleType::Text => {
                load_typed_module_record(scope, &base, &specifier, kind, false, &policy)?
            }
            crate::ModuleType::Css => {
                throw_type_error(scope, "CSS modules are unavailable in worker globals");
                return None;
            }
        };
        if module.get_status() == v8::ModuleStatus::Uninstantiated
            && module.instantiate_module(scope, agent_resolve_module) != Some(true)
        {
            return None;
        }
        if module.get_status() == v8::ModuleStatus::Errored {
            let error = module.get_exception();
            return Some(PreparedModuleImport::Rejection(v8::Global::new(
                scope, error,
            )));
        }
        let evaluation = evaluate_import_phase(scope, module, phase)?;
        let namespace = module.get_module_namespace_with_phase(phase);
        Some(PreparedModuleImport::Evaluation {
            namespace: v8::Global::new(scope, namespace),
            promise: v8::Global::new(scope, evaluation),
        })
    })();
    if let Some(failure) = take_module_preparation_failure(scope) {
        return Some(PreparedModuleImport::NativeFailure(failure));
    }
    if loaded.is_none() {
        // Empty without a JavaScript exception is infrastructure suspension.
        // The native import owner retries against the cached evaluation Promise.
        let error = scope.exception()?;
        scope.reset();
        return Some(PreparedModuleImport::Rejection(v8::Global::new(
            scope, error,
        )));
    }
    loaded
}

#[cfg(test)]
mod agent_runtime_tests {
    use super::*;
    use crate::agent_registry::AgentProcess;
    use crate::agent_thread::AgentThreadProcess;
    use oryn_js::worker_wire::*;
    type BlobRegistry = (u64, BTreeMap<String, (Vec<u8>, String)>);

    struct Backend {
        blobs: Mutex<BlobRegistry>,
        hub: Arc<Mutex<oryn_js::message_wire::MessageHub>>,
        scripts: BTreeMap<String, Vec<u16>>,
    }
    impl WorkerTransport for Backend {
        fn request(
            &self,
            request: AgentServiceRequest,
        ) -> Result<AgentServiceResponse, WorkerError> {
            match request {
                // This backend has no policies that emit reports, but polling is
                // still part of every real worker checkpoint.
                AgentServiceRequest::CspEvents { .. } => {
                    Ok(AgentServiceResponse::CspEvents(Vec::new()))
                }
                AgentServiceRequest::BlobRegister {
                    creator,
                    bytes,
                    media_type,
                } => {
                    if creator != Creator::Agent(AgentId(1)) {
                        return Err(WorkerError::new(
                            "SecurityError",
                            "foreign blob creator route",
                        ));
                    }
                    let mut blobs = self
                        .blobs
                        .lock()
                        .map_err(|_| agent_error("blob registry poisoned"))?;
                    blobs.0 = blobs
                        .0
                        .checked_add(1)
                        .ok_or_else(|| agent_error("blob token exhausted"))?;
                    let url = format!("blob:https://a.test/fixture-{}", blobs.0);
                    blobs.1.insert(url.clone(), (bytes, media_type));
                    Ok(AgentServiceResponse::BlobUrl(url))
                }
                AgentServiceRequest::BlobResolve { creator, url } => {
                    if creator != Creator::Agent(AgentId(1)) {
                        return Err(WorkerError::new(
                            "SecurityError",
                            "foreign blob creator route",
                        ));
                    }
                    let blobs = self
                        .blobs
                        .lock()
                        .map_err(|_| agent_error("blob registry poisoned"))?;
                    let (bytes, media_type) = blobs
                        .1
                        .get(&url)
                        .ok_or_else(|| WorkerError::new("NetworkError", "blob URL unavailable"))?;
                    Ok(AgentServiceResponse::Blob {
                        bytes: bytes.clone(),
                        media_type: media_type.clone(),
                    })
                }
                AgentServiceRequest::BlobRevoke { creator, url } => {
                    if creator != Creator::Agent(AgentId(1)) {
                        return Err(WorkerError::new(
                            "SecurityError",
                            "foreign blob creator route",
                        ));
                    }
                    self.blobs
                        .lock()
                        .map_err(|_| agent_error("blob registry poisoned"))?
                        .1
                        .remove(&url);
                    Ok(AgentServiceResponse::Done)
                }
                AgentServiceRequest::Message(request) => {
                    let mut hub = self.hub.lock().unwrap();
                    hub.set_partition(
                        oryn_js::message_wire::Principal::Agent(1),
                        "context/site".into(),
                    );
                    hub.handle_agent(1, request, "https://a.test", "oryn:origin:https://a.test")
                        .map(AgentServiceResponse::Message)
                        .map_err(agent_error)
                }
                AgentServiceRequest::ImportScript { url, .. }
                | AgentServiceRequest::ImportModule { url, .. }
                | AgentServiceRequest::ImportWorkletModule { url, .. } => self
                    .scripts
                    .get(&url)
                    .map(|source| AgentServiceResponse::Script {
                        url,
                        source: source.clone(),
                        referrer_policy: String::new(),
                    })
                    .ok_or_else(|| WorkerError::new("NetworkError", "missing script")),
                _ => Err(agent_error("unexpected agent service request")),
            }
        }
    }
    #[test]
    fn fixture_worker_services_poll_and_revoke_exact_creator_blob_bytes() {
        let backend = Backend {
            blobs: Default::default(),
            hub: Default::default(),
            scripts: Default::default(),
        };
        for after in [0, 17, u64::MAX] {
            assert!(
                matches!(backend.request(AgentServiceRequest::CspEvents{after}).unwrap(),AgentServiceResponse::CspEvents(events) if events.is_empty())
            );
        }
        let creator = Creator::Agent(AgentId(1));
        let AgentServiceResponse::BlobUrl(url) = backend
            .request(AgentServiceRequest::BlobRegister {
                creator: creator.clone(),
                bytes: vec![0, 255, 1],
                media_type: "application/octet-stream".into(),
            })
            .unwrap()
        else {
            panic!("blob receipt")
        };
        assert!(
            matches!(backend.request(AgentServiceRequest::BlobResolve{creator:creator.clone(),url:url.clone()}).unwrap(),AgentServiceResponse::Blob{bytes,media_type} if bytes==vec![0,255,1]&&media_type=="application/octet-stream")
        );
        assert!(
            backend
                .request(AgentServiceRequest::BlobRevoke {
                    creator: Creator::Agent(AgentId(2)),
                    url: url.clone()
                })
                .is_err()
        );
        assert!(matches!(
            backend
                .request(AgentServiceRequest::BlobResolve {
                    creator: creator.clone(),
                    url: url.clone()
                })
                .unwrap(),
            AgentServiceResponse::Blob { .. }
        ));
        for _ in 0..2 {
            assert!(matches!(
                backend
                    .request(AgentServiceRequest::BlobRevoke {
                        creator: creator.clone(),
                        url: url.clone()
                    })
                    .unwrap(),
                AgentServiceResponse::Done
            ));
        }
        assert!(
            backend
                .request(AgentServiceRequest::BlobResolve { creator, url })
                .is_err()
        );
        assert!(backend.blobs.lock().unwrap().1.is_empty());
    }
    fn bootstrap(source: &str, kind: AgentKind, script_type: ScriptType) -> AgentBootstrap {
        AgentBootstrap {
            cookie_ancestor_urls: Vec::new(),
            import_metadata: Default::default(),
            service_skip_waiting: false,
            resource_digests: Default::default(),
            secure_context: true,
            service_registration: None,
            service_controller: None,
            policy: oryn_js::worker_wire::AgentPolicy::default(),
            top_level_url: None,
            id: AgentId(1),
            kind,
            partition: "context/site".into(),
            origin: AgentOrigin {
                serialized: "https://a.test".into(),
                authority: "oryn:origin:https://a.test".into(),
                opaque: false,
            },
            script_url: "https://a.test/worker.js".into(),
            source: source.encode_utf16().collect(),
            modules: BTreeMap::new(),
            options: WorkerOptions {
                script_type,
                ..Default::default()
            },
            referrer_policy: String::new(),
            limits: AgentLimits {
                task_wall_ms: 30_000,
                ..Default::default()
            },
        }
    }
    #[test]
    fn worker_startup_graph_yields_for_javascript_json_text_and_cancels_before_ready() {
        struct Held {
            inner: Backend,
            gate: Arc<(Mutex<bool>, std::sync::Condvar)>,
            stopped: AtomicBool,
            entered: std::sync::mpsc::Sender<ImportedModuleType>,
        }
        impl WorkerTransport for Held {
            fn retire(&self) {
                self.stopped.store(true, Ordering::Release);
                let (gate, wake) = (&self.gate.0, &self.gate.1);
                let mut gate = gate.lock().unwrap();
                *gate = true;
                wake.notify_all();
            }
            fn request(
                &self,
                request: AgentServiceRequest,
            ) -> Result<AgentServiceResponse, WorkerError> {
                if let AgentServiceRequest::ImportModule { url, kind, .. } = request {
                    self.entered.send(kind).unwrap();
                    let mut ready = self.gate.0.lock().unwrap();
                    while !*ready {
                        ready = self.gate.1.wait(ready).unwrap();
                    }
                    if self.stopped.load(Ordering::Acquire) {
                        return Err(WorkerError::new("AbortError", "startup retired"));
                    }
                    let source = match kind {
                        ImportedModuleType::Text => "text file\n",
                        ImportedModuleType::Json => "{\"value\":42}",
                        ImportedModuleType::JavaScript => "globalThis.side=1;export {};",
                    };
                    Ok(AgentServiceResponse::Script {
                        url,
                        source: source.encode_utf16().collect(),
                        referrer_policy: "no-referrer".into(),
                    })
                } else {
                    self.inner.request(request)
                }
            }
        }
        let make = || {
            let (entered, observed) = std::sync::mpsc::channel();
            let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
            let backend = Arc::new(Held {
                inner: Backend {
                    blobs: Default::default(),
                    hub: Arc::new(Mutex::new(Default::default())),
                    scripts: BTreeMap::new(),
                },
                gate: gate.clone(),
                stopped: AtomicBool::new(false),
                entered,
            });
            (backend, gate, observed)
        };
        let (backend, gate, observed) = make();
        let source = "import data from './value.json' with {type:'json'};import text from './text' with {type:'text'};import './side.js';globalThis.answer=data.value;globalThis.text=text;";
        let mut runtime = NativeAgentRuntime::create(
            bootstrap(source, AgentKind::Dedicated, ScriptType::Module),
            backend,
        )
        .unwrap();
        runtime.start().unwrap();
        assert!(runtime.startup.is_some());
        assert!(runtime.evaluation.is_none());
        observed.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(
            runtime
                .host
                .evaluate_string("typeof answer+'|'+typeof side")
                .unwrap(),
            "undefined|undefined"
        );
        {
            *gate.0.lock().unwrap() = true;
            gate.1.notify_all();
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while runtime.startup.is_some() {
            runtime.resume_startup().unwrap();
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(
            runtime
                .host
                .evaluate_string("answer+'|'+side+'|'+text")
                .unwrap(),
            "42|1|text file\n"
        );
        drop(runtime);
        let (backend, _gate, observed) = make();
        let boot = bootstrap(
            "import './held.js';postMessage('must not run')",
            AgentKind::Dedicated,
            ScriptType::Module,
        );
        let mut process =
            AgentThreadProcess::spawn(boot, move |boot| NativeAgentRuntime::create(boot, backend))
                .unwrap();
        observed.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(
            !process
                .poll(32)
                .unwrap()
                .iter()
                .any(|event| matches!(event, AgentEvent::Ready | AgentEvent::Message { .. }))
        );
        let before = Instant::now();
        process.terminate().unwrap();
        assert!(before.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn worker_close_preserves_current_turn_but_discards_following_tasks() {
        // importScripts after close exercises the broker route: closing task
        // admission is distinct from retiring API/capability authority.
        for (source, ready) in [
            (
                "setTimeout(()=>postMessage('late'),0);close();importScripts('./after-close.js');postMessage('current');Promise.resolve().then(()=>postMessage('microtask'));",
                false,
            ),
            (
                "setTimeout(()=>{close();importScripts('./after-close.js');postMessage('current');Promise.resolve().then(()=>postMessage('microtask'))},0);setTimeout(()=>postMessage('late'),0);",
                true,
            ),
        ] {
            let scripts = BTreeMap::from([(
                "https://a.test/after-close.js".into(),
                "globalThis.importedAfterClose=true"
                    .encode_utf16()
                    .collect(),
            )]);
            let mut process = launch(
                bootstrap(source, AgentKind::Dedicated, ScriptType::Classic),
                scripts,
            );
            let mut values = Vec::new();
            let mut saw_ready = false;
            let mut closed = false;
            let deadline = Instant::now() + Duration::from_secs(5);
            while !closed {
                for event in process.poll(32).unwrap() {
                    match event {
                        AgentEvent::Ready => saw_ready = true,
                        AgentEvent::Message { value, .. } => values.push(value),
                        AgentEvent::Closed => closed = true,
                        event => panic!("unexpected close event: {event:?}"),
                    }
                }
                assert!(Instant::now() < deadline, "worker close deadline");
                std::thread::yield_now();
            }
            assert_eq!(saw_ready, ready);
            assert_eq!(
                values.len(),
                2,
                "only the current task and its checkpoint may publish"
            );
            verify(values.remove(0), "received==='current'");
            verify(values.remove(0), "received==='microtask'");
            assert!(process.poll(32).unwrap().is_empty());
            process.terminate().unwrap();
        }
    }

    #[test]
    fn observable_is_installed_in_each_worker_global() {
        for kind in [AgentKind::Dedicated, AgentKind::Shared, AgentKind::Service] {
            let backend = Arc::new(Backend {
                blobs: Default::default(),
                hub: Arc::new(Mutex::new(Default::default())),
                scripts: BTreeMap::new(),
            });
            let mut runtime = NativeAgentRuntime::create(
                bootstrap("", kind, ScriptType::Classic), backend,
            ).unwrap();
            runtime.start().unwrap();
            runtime.host.execute(r#"
                const values=[];
                const observable=new Observable(subscriber=>{
                    subscriber.addTeardown(()=>values.push('teardown'));
                    subscriber.next(7);subscriber.complete();subscriber.next(8);
                });
                observable.subscribe({next:value=>values.push(value),complete:()=>values.push('complete')});
                if(values.join(',')!=='7,teardown,complete')throw Error('worker Observable lifecycle');
            "#).unwrap();
        }
    }

    #[test]
    fn dedicated_and_shared_close_keep_current_callbacks_without_later_timer_admission() {
        for kind in [AgentKind::Dedicated, AgentKind::Shared] {
            let backend = Arc::new(Backend {
                blobs: Default::default(),
                hub: Arc::new(Mutex::new(Default::default())),
                scripts: BTreeMap::new(),
            });
            let mut runtime=NativeAgentRuntime::create(bootstrap("globalThis.steps=[];setTimeout(()=>{steps.push('first');close();steps.push('after');Promise.resolve().then(()=>steps.push('microtask'))},0);setTimeout(()=>steps.push('late'),0)",kind,ScriptType::Classic),backend).unwrap();
            runtime.start().unwrap();
            runtime.host.drain_scheduled_timers(false).unwrap();
            assert_eq!(
                runtime.host.evaluate_string("steps.join(',')").unwrap(),
                "first,after,microtask"
            );
            assert!(runtime.host.worker_tasks_stopped());
            assert_eq!(runtime.host.drain_scheduled_timers(false).unwrap(), 0);
            assert_eq!(runtime.host.drain_async_fetches(false).unwrap(), 0);
            let state = runtime.host.binding_states[&runtime.host.active_realm].clone();
            assert!(state.execution_tasks_stopped());
            assert!(
                !state.execution_retired(),
                "close must not revoke an in-progress callback's realm"
            );
        }
    }

    #[test]
    fn worker_author_exceptions_forward_metadata_without_killing_scheduled_work() {
        let source = r#"addEventListener('probe',()=>{throw Error('listener')});dispatchEvent(new Event('probe'));
            setTimeout(()=>{throw Error('timer')},0);
            setTimeout(()=>{postMessage('alive');close()},0);
            throw Error('startup\ud800');"#;
        let mut process = launch(
            bootstrap(source, AgentKind::Dedicated, ScriptType::Classic),
            BTreeMap::new(),
        );
        let mut errors = Vec::new();
        let mut values = Vec::new();
        let mut closed = false;
        let deadline = Instant::now() + Duration::from_secs(5);
        while !closed {
            for event in process.poll(32).unwrap() {
                match event {
                    AgentEvent::Error {
                        message,
                        filename,
                        line,
                        column,
                    } => {
                        assert_eq!(filename, "https://a.test/worker.js");
                        assert!(line > 0 && column > 0);
                        errors.push(message);
                    }
                    AgentEvent::Message { value, .. } => values.push(value),
                    AgentEvent::Closed => closed = true,
                    _ => {}
                }
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(errors.len(), 3);
        assert!(String::from_utf16_lossy(&errors[0]).contains("listener"));
        assert!(errors[1].contains(&0xd800));
        assert!(String::from_utf16_lossy(&errors[2]).contains("timer"));
        assert_eq!(values.len(), 1);
        verify(values.remove(0), "received==='alive'");
        process.terminate().unwrap();
    }

    #[test]
    fn worker_error_cancellation_and_nested_handler_exception_have_distinct_reports() {
        for (source, expected) in [
            (
                "onerror=(message,file,line,column,error)=>{if(!(error instanceof Error)||line===0||column===0)throw Error('metadata');return true};setTimeout(()=>{postMessage('alive');close()},0);throw Error('handled')",
                0,
            ),
            (
                "addEventListener('error',()=>{throw Error('handler')});setTimeout(()=>{postMessage('alive');close()},0);throw Error('original')",
                2,
            ),
        ] {
            let mut process = launch(
                bootstrap(source, AgentKind::Dedicated, ScriptType::Classic),
                BTreeMap::new(),
            );
            let mut errors = Vec::new();
            let mut alive = false;
            let mut closed = false;
            let deadline = Instant::now() + Duration::from_secs(5);
            while !closed {
                for event in process.poll(32).unwrap() {
                    match event {
                        AgentEvent::Error { message, .. } => {
                            errors.push(String::from_utf16_lossy(&message))
                        }
                        AgentEvent::Message { value, .. } => {
                            verify(value, "received==='alive'");
                            alive = true;
                        }
                        AgentEvent::Closed => closed = true,
                        _ => {}
                    }
                }
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            }
            assert_eq!(errors.len(), expected);
            if expected == 2 {
                assert!(errors[0].contains("handler"));
                assert!(errors[1].contains("original"));
            }
            assert!(alive);
            process.terminate().unwrap();
        }
    }

    #[test]
    fn module_worker_rejections_report_once_and_keep_existing_timers() {
        for source in [
            "setTimeout(()=>{postMessage('alive');close()},0);throw Error('module failure')",
            "setTimeout(()=>{postMessage('alive');close()},5);await Promise.resolve();throw Error('module failure')",
        ] {
            let mut process = launch(
                bootstrap(source, AgentKind::Dedicated, ScriptType::Module),
                BTreeMap::new(),
            );
            let mut errors = 0;
            let mut alive = false;
            let mut closed = false;
            let deadline = Instant::now() + Duration::from_secs(5);
            while !closed {
                for event in process.poll(32).unwrap() {
                    match event {
                        AgentEvent::Error { message, .. } => {
                            assert!(String::from_utf16_lossy(&message).contains("module failure"));
                            errors += 1;
                        }
                        AgentEvent::Message { value, .. } => {
                            verify(value, "received==='alive'");
                            alive = true;
                        }
                        AgentEvent::Closed => closed = true,
                        _ => {}
                    }
                }
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            }
            assert_eq!(errors, 1);
            assert!(alive);
            process.terminate().unwrap();
        }
    }

    #[test]
    fn service_startup_failure_never_readies_and_cancellation_only_suppresses_reporting() {
        for (script_type, body) in [
            (
                ScriptType::Classic,
                "throw Error('original startup failure')",
            ),
            (
                ScriptType::Module,
                "throw Error('original startup failure');export {}",
            ),
            (
                ScriptType::Module,
                "await new Promise(resolve=>setTimeout(resolve,0));throw Error('original startup failure')",
            ),
        ] {
            for cancel in [false, true] {
                let source = format!(
                    "{};{body}",
                    if cancel {
                        "addEventListener('error',event=>event.preventDefault())"
                    } else {
                        ""
                    }
                );
                let mut process = launch(
                    bootstrap(&source, AgentKind::Service, script_type),
                    BTreeMap::new(),
                );
                let mut reports = 0;
                let deadline = Instant::now() + Duration::from_secs(10);
                let failure = loop {
                    match process.poll(1) {
                        Ok(events) => {
                            for event in events {
                                match event {
                                    AgentEvent::Error { message, .. } => {
                                        assert!(
                                            String::from_utf16_lossy(&message)
                                                .contains("original startup failure")
                                        );
                                        reports += 1;
                                    }
                                    event => panic!("failed startup emitted {event:?}"),
                                }
                            }
                        }
                        Err(error) => break error,
                    }
                    assert!(
                        Instant::now() < deadline,
                        "service startup failure deadline"
                    );
                    std::thread::yield_now();
                };
                assert!(failure.message.contains("original startup failure"));
                assert_eq!(reports, if cancel { 0 } else { 1 });
                process.terminate().unwrap();
            }
        }
    }

    #[test]
    fn service_oversized_startup_report_has_bounded_retry_and_original_failure() {
        let mut boot = bootstrap(
            "throw Error('original oversized startup failure '+ 'x'.repeat(16384))",
            AgentKind::Service,
            ScriptType::Classic,
        );
        boot.limits.queue_bytes = 512;
        boot.limits.task_wall_ms = 500;
        let mut process = launch(boot, BTreeMap::new());
        let started = Instant::now();
        let failure = loop {
            match process.poll(1) {
                Ok(events) => assert!(
                    events.is_empty(),
                    "oversized failure cannot Ready or partially publish"
                ),
                Err(error) => break error,
            }
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "permanent report publication failure must not hang startup"
            );
            std::thread::yield_now();
        };
        assert!(
            failure
                .message
                .contains("original oversized startup failure")
        );
        assert!(failure.message.ends_with(" [truncated]"));
        assert!(failure.message.len() < 32 * 1024);
        process.terminate().unwrap();
    }

    #[test]
    fn service_timer_error_while_module_evaluation_pending_precedes_ready() {
        let source = "await new Promise(resolve=>{setTimeout(()=>{throw Error('independent timer failure')},0);setTimeout(resolve,10)});";
        let mut process = launch(
            bootstrap(source, AgentKind::Service, ScriptType::Module),
            BTreeMap::new(),
        );
        let mut reports = 0;
        let mut ready = false;
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready {
            for event in process.poll(1).unwrap() {
                match event {
                    AgentEvent::Error { message, .. } => {
                        assert!(
                            String::from_utf16_lossy(&message)
                                .contains("independent timer failure")
                        );
                        reports += 1;
                    }
                    AgentEvent::Ready => {
                        assert_eq!(reports, 1);
                        ready = true;
                    }
                    event => panic!("unexpected pending startup event {event:?}"),
                }
            }
            assert!(Instant::now() < deadline, "service module ready deadline");
            std::thread::yield_now();
        }
        process.terminate().unwrap();
    }

    #[test]
    fn service_startup_keeps_original_failure_and_report_after_transient_publication_error() {
        for script_type in [ScriptType::Classic, ScriptType::Module] {
            let backend = Arc::new(Backend {
                blobs: Default::default(),
                hub: Default::default(),
                scripts: Default::default(),
            });
            let mut runtime=NativeAgentRuntime::create(bootstrap("globalThis.reports=0;addEventListener('error',()=>reports++);throw Error('original retained failure')",AgentKind::Service,script_type),backend).unwrap();
            let state = runtime.host.binding_states[&runtime.host.active_realm].clone();
            let attempts = Rc::new(std::cell::Cell::new(0));
            let observed = attempts.clone();
            state.set_worker_error_reporter(Rc::new(move |event| {
                assert!(matches!(event, AgentEvent::Error { .. }));
                let count = observed.get();
                observed.set(count + 1);
                if count == 0 {
                    Err("transient publication failure".into())
                } else {
                    Ok(())
                }
            }));
            runtime.start().unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while runtime.startup.is_some() {
                runtime.resume_startup().unwrap();
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            }
            assert!(
                runtime
                    .service_startup_failure
                    .as_ref()
                    .unwrap()
                    .message
                    .contains("original retained failure")
            );
            assert!(state.has_pending_script_error_reports());
            assert_eq!(attempts.get(), 1);
            while state.has_pending_script_error_reports() {
                {
                    v8::scope!(let handles,&mut runtime.host.isolate);
                    let context =
                        v8::Local::new(handles, &runtime.host.contexts[&runtime.host.active_realm]);
                    let scope = &mut v8::ContextScope::new(handles, context);
                    state.poll_script_error_reports(scope).unwrap();
                }
                assert!(Instant::now() < deadline);
                std::thread::yield_now();
            }
            assert_eq!(attempts.get(), 2);
            assert_eq!(
                runtime.host.evaluate_string("String(reports)").unwrap(),
                "1"
            );
            assert!(
                runtime
                    .service_startup_failure
                    .as_ref()
                    .unwrap()
                    .message
                    .contains("original retained failure")
            );
        }
    }

    #[test]
    fn parent_worker_response_policy_controls_eval_without_author_override() {
        let mut boot = bootstrap(
            "let denied=false;try{eval('1')}catch(e){denied=e.name==='EvalError'};postMessage(denied);close()",
            AgentKind::Dedicated,
            ScriptType::Classic,
        );
        boot.policy = oryn_js::worker_wire::AgentPolicy {
            hide_nonces: false,
            policy_sources: Vec::new(),
            policy_self_urls: Vec::new(),
            document_self_url: None,
            policy_self_url: None,
            url: "https://a.test/worker.js".into(),
            headers: vec![("content-security-policy".into(), "script-src 'self'".into())],
        };
        let mut process = launch(boot, BTreeMap::new());
        verify(message(&mut process), "received === true");
        process.terminate().unwrap();
    }
    #[test]
    fn worker_event_burst_does_not_use_page_command_queue_capacity() {
        let browser = crate::runtime::Browser::in_process_with_config(crate::runtime::BrowserConfig {
            worker_queue_capacity: 2,
            ..Default::default()
        });
        let context = browser.new_context();
        let mut boot = bootstrap(
            "for(let i=0;i<64;i++)postMessage(i);close();",
            AgentKind::Dedicated,
            ScriptType::Classic,
        );
        boot.limits = context.agent_limits();
        let mut process = launch(boot, BTreeMap::new());
        // Let the synchronous startup task finish before polling: the page's
        // command queue must not truncate an independent worker's event burst.
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut values = Vec::new();
        std::thread::sleep(Duration::from_millis(100));
        loop {
            let mut closed = false;
            for event in process.poll(32).unwrap() {
                match event {
                    AgentEvent::Message { value, .. } => values.push(value),
                    AgentEvent::Closed => closed = true,
                    AgentEvent::Ready => {},
                    event => panic!("unexpected worker burst event: {event:?}"),
                }
            }
            if closed { break; }
            assert!(Instant::now() < deadline, "worker burst must settle");
            std::thread::sleep(Duration::from_millis(1));
        }
        process.terminate().unwrap();
        assert_eq!(values.len(), 64);
        for (index, value) in values.into_iter().enumerate() {
            verify(value, &format!("received === {index}"));
        }
    }
    fn launch(
        bootstrap: AgentBootstrap,
        scripts: BTreeMap<String, Vec<u16>>,
    ) -> AgentThreadProcess {
        let backend = Arc::new(Backend {
            blobs: Default::default(),
            hub: Arc::new(Mutex::new(Default::default())),
            scripts,
        });
        AgentThreadProcess::spawn(bootstrap, move |bootstrap| {
            NativeAgentRuntime::create(bootstrap, backend)
        })
        .unwrap()
    }
    fn message(process: &mut AgentThreadProcess) -> oryn_js::message_wire::WireValue {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            for event in process.poll(32).unwrap() {
                if let AgentEvent::Message { value, .. } = event {
                    return value;
                }
            }
            assert!(
                Instant::now() < deadline,
                "independent worker message deadline"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn verify(value: oryn_js::message_wire::WireValue, expression: &str) {
        let mut target = RawV8Host::new();
        target
            .install_dom(
                Rc::new(RefCell::new(
                    oryn_dom::html::parse("<body>").into_runtime().unwrap(),
                )),
                1,
            )
            .unwrap();
        {
            v8::scope!(let handles,&mut target.isolate);
            let context = v8::Local::new(handles, &target.contexts[&target.active_realm]);
            let scope = &mut v8::ContextScope::new(handles, context);
            let (value, _) =
                oryn_js::v8_bindings::structured_clone::deserialize_wire(scope, value).unwrap();
            let key = v8::String::new(scope, "received").unwrap();
            context
                .global(scope)
                .create_data_property(scope, key.into(), value);
        }
        assert_eq!(target.evaluate_string(expression).unwrap(), "true");
    }
    #[test]
    fn independent_worker_preserves_graphs_utf16_and_autonomous_timers() {
        let source = r#"JSON.stringify=JSON.parse=()=>{throw Error('poisoned JSON')};
            setTimeout(()=>{const d={big:123n,text:'\ud800',buffer:new ArrayBuffer(8)};d.self=d;d.map=new Map([[d,d.buffer]]);postMessage(d,[d.buffer]);close()},0);"#;
        let mut process = launch(
            bootstrap(source, AgentKind::Dedicated, ScriptType::Classic),
            BTreeMap::new(),
        );
        let value = message(&mut process);
        verify(
            value,
            "String(received.self===received&&received.big===123n&&received.text.charCodeAt(0)===0xd800&&received.map instanceof Map&&received.map.get(received)===received.buffer&&received.buffer.byteLength===8)",
        );
        process.terminate().unwrap();
    }
    #[test]
    fn worker_source_cannot_replace_native_import_base_or_escape_agent_route() {
        let mut scripts = BTreeMap::new();
        scripts.insert(
            "https://a.test/dependency.js".into(),
            "globalThis.answer=91".encode_utf16().collect(),
        );
        let mut process = launch(
            bootstrap(
                "globalThis.location={href:'https://foreign.test/'};importScripts('dependency.js');postMessage({answer,legacy:typeof __oryn_worker_create,context:typeof __oryn_context_call});close()",
                AgentKind::Dedicated,
                ScriptType::Classic,
            ),
            scripts,
        );
        verify(
            message(&mut process),
            "String(received.answer===91&&received.legacy==='undefined'&&received.context==='undefined')",
        );
        process.terminate().unwrap();
    }
    #[test]
    fn worker_json_imports_share_native_exports_and_reject_css() {
        let mut scripts = BTreeMap::new();
        scripts.insert(
            "https://a.test/data.json".into(),
            "{\"value\":42}".encode_utf16().collect(),
        );
        let source = r#"import data from './data.json' with {type:'json'};
            JSON.parse=()=>{throw Error('author JSON.parse')};
            const again=await import('./data.json',{with:{type:'json'}});
            let denied=false;try{await import('./sheet.css',{with:{type:'css'}})}catch(e){denied=e instanceof TypeError}
            postMessage([data.value,again.default===data,denied]);close();"#;
        let mut process = launch(
            bootstrap(source, AgentKind::Dedicated, ScriptType::Module),
            scripts,
        );
        verify(
            message(&mut process),
            "received[0]===42&&received[1]===true&&received[2]===true",
        );
        process.terminate().unwrap();
    }

    #[test]
    fn service_worker_dynamic_import_rejects_before_transport() {
        let mut runtime = inline(
            AgentKind::Service,
            "globalThis.denied=false;import('./missing.js').catch(error=>denied=error instanceof TypeError)",
        );
        assert_eq!(
            runtime.host.evaluate_string("String(denied)").unwrap(),
            "true"
        );
    }

    #[test]
    fn module_worker_self_import_shares_initial_evaluation_without_fetch() {
        let source = "globalThis.count=(globalThis.count||0)+1;import('./worker.js').then(m=>{postMessage([count,m.value]);close()});export const value=42;";
        let mut process = launch(
            bootstrap(source, AgentKind::Dedicated, ScriptType::Module),
            BTreeMap::new(),
        );
        verify(message(&mut process), "received[0]===1&&received[1]===42");
        process.terminate().unwrap();
    }

    #[test]
    fn worker_import_retries_local_failure_but_retains_established_fetch_failure() {
        struct FailureBackend {
            inner: Backend,
            calls: Arc<Mutex<usize>>,
            kind: Option<ScriptFailureKind>,
        }
        impl WorkerTransport for FailureBackend {
            fn request(
                &self,
                request: AgentServiceRequest,
            ) -> Result<AgentServiceResponse, WorkerError> {
                match request {
                    AgentServiceRequest::ImportModule { url, .. } => {
                        let mut calls = self.calls.lock().unwrap();
                        *calls += 1;
                        if *calls == 1 {
                            let mut error = WorkerError::new(
                                "NetworkError",
                                "same text for every native failure class",
                            );
                            error.script_failure = self.kind;
                            return Err(error);
                        }
                        Ok(AgentServiceResponse::Script {
                            url,
                            source: "export const value=42".encode_utf16().collect(),
                            referrer_policy: String::new(),
                        })
                    }
                    other => self.inner.request(other),
                }
            }
        }
        for kind in [
            None,
            Some(ScriptFailureKind::Infrastructure),
            Some(ScriptFailureKind::Cancelled),
            Some(ScriptFailureKind::Capacity),
            Some(ScriptFailureKind::Fetch),
        ] {
            let calls = Arc::new(Mutex::new(0));
            let backend = Arc::new(FailureBackend {
                inner: Backend {
                    blobs: Default::default(),
                    hub: Arc::new(Mutex::new(Default::default())),
                    scripts: BTreeMap::new(),
                },
                calls: calls.clone(),
                kind,
            });
            let source = "try{await import('./retry.js')}catch(e){}let value=0;try{value=(await import('./retry.js')).value}catch(e){}postMessage(value);close()";
            let boot = bootstrap(source, AgentKind::Dedicated, ScriptType::Module);
            let mut process = AgentThreadProcess::spawn(boot, move |boot| {
                NativeAgentRuntime::create(boot, backend)
            })
            .unwrap();
            let sticky = kind == Some(ScriptFailureKind::Fetch);
            verify(
                message(&mut process),
                if sticky {
                    "received===0"
                } else {
                    "received===42"
                },
            );
            process.terminate().unwrap();
            assert_eq!(*calls.lock().unwrap(), if sticky { 1 } else { 2 });
        }
    }

    #[test]
    fn worker_redirect_aliases_keep_requested_identity_and_final_import_meta() {
        struct AliasBackend {
            inner: Backend,
            seen: Arc<Mutex<Vec<String>>>,
        }
        impl WorkerTransport for AliasBackend {
            fn request(
                &self,
                request: AgentServiceRequest,
            ) -> Result<AgentServiceResponse, WorkerError> {
                match request {
                    AgentServiceRequest::ImportModule { url, .. } => {
                        self.seen.lock().unwrap().push(url);
                        Ok(AgentServiceResponse::Script{url:"https://a.test/final.js".into(),
                            source:"globalThis.evaluations=(globalThis.evaluations||0)+1;export const ordinal=globalThis.evaluations;export const url=import.meta.url".encode_utf16().collect(),referrer_policy:String::new()})
                    }
                    other => self.inner.request(other),
                }
            }
        }
        let seen = Arc::new(Mutex::new(Vec::new()));
        let backend = Arc::new(AliasBackend {
            inner: Backend {
                blobs: Default::default(),
                hub: Arc::new(Mutex::new(Default::default())),
                scripts: BTreeMap::new(),
            },
            seen: seen.clone(),
        });
        let source = "const a=await import('./a.js');const b=await import('./b.js');const direct=await import('./final.js');const again=await import('./a.js');postMessage(a!==b&&b!==direct&&a===again&&a.ordinal===1&&b.ordinal===2&&direct.ordinal===3&&a.url===b.url&&b.url===direct.url);close()";
        let boot = bootstrap(source, AgentKind::Dedicated, ScriptType::Module);
        let mut process =
            AgentThreadProcess::spawn(boot, move |boot| NativeAgentRuntime::create(boot, backend))
                .unwrap();
        verify(message(&mut process), "received===true");
        process.terminate().unwrap();
        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                "https://a.test/a.js",
                "https://a.test/b.js",
                "https://a.test/final.js"
            ]
        );
    }

    #[test]
    fn redirected_worker_modules_keep_each_response_referrer_policy() {
        struct PolicyBackend {
            inner: Backend,
            policies: Arc<Mutex<Vec<String>>>,
        }
        impl WorkerTransport for PolicyBackend {
            fn request(
                &self,
                request: AgentServiceRequest,
            ) -> Result<AgentServiceResponse, WorkerError> {
                match request {
                    AgentServiceRequest::ImportModule {
                        url,
                        referrer_policy,
                        ..
                    } => {
                        if url.contains("leaf-") {
                            self.policies.lock().unwrap().push(referrer_policy);
                            return Ok(AgentServiceResponse::Script {
                                url,
                                source: "export default 1".encode_utf16().collect(),
                                referrer_policy: String::new(),
                            });
                        }
                        let (suffix, policy) = if url.ends_with("a.js") {
                            ("a", "no-referrer")
                        } else {
                            ("b", "origin")
                        };
                        let source = format!(
                            "export function load(){{return import('./leaf-{suffix}.js')}}"
                        );
                        Ok(AgentServiceResponse::Script {
                            url: "https://a.test/redirect/shared.js".into(),
                            source: source.encode_utf16().collect(),
                            referrer_policy: policy.into(),
                        })
                    }
                    other => self.inner.request(other),
                }
            }
        }
        let policies = Arc::new(Mutex::new(Vec::new()));
        let backend = Arc::new(PolicyBackend {
            inner: Backend {
                blobs: Default::default(),
                hub: Arc::new(Mutex::new(Default::default())),
                scripts: BTreeMap::new(),
            },
            policies: policies.clone(),
        });
        let source = "const a=await import('./a.js');const b=await import('./b.js');await a.load();await b.load();postMessage(true);close()";
        let boot = bootstrap(source, AgentKind::Dedicated, ScriptType::Module);
        let mut process =
            AgentThreadProcess::spawn(boot, move |boot| NativeAgentRuntime::create(boot, backend))
                .unwrap();
        verify(message(&mut process), "received===true");
        process.terminate().unwrap();
        assert_eq!(*policies.lock().unwrap(), vec!["no-referrer", "origin"]);
    }

    #[test]
    fn worker_module_redirect_uses_final_base_and_reuses_fetch() {
        struct RedirectBackend {
            inner: Backend,
            seen: Arc<Mutex<Vec<String>>>,
        }
        impl WorkerTransport for RedirectBackend {
            fn request(
                &self,
                request: AgentServiceRequest,
            ) -> Result<AgentServiceResponse, WorkerError> {
                match request {
                    AgentServiceRequest::ImportModule {
                        url,
                        kind: ImportedModuleType::JavaScript,
                        ..
                    } => {
                        self.seen.lock().unwrap().push(url.clone());
                        let (url, source) = match url.as_str() {
                            "https://a.test/start.js" => (
                                "https://a.test/final/main.js",
                                "import {value} from './child.js';export {value};",
                            ),
                            "https://a.test/final/child.js" => {
                                ("https://a.test/final/child.js", "export const value=42;")
                            }
                            _ => {
                                return Err(WorkerError::new(
                                    "NetworkError",
                                    "incorrect module base",
                                ));
                            }
                        };
                        Ok(AgentServiceResponse::Script {
                            url: url.into(),
                            source: source.encode_utf16().collect(),
                            referrer_policy: "no-referrer".into(),
                        })
                    }
                    other => self.inner.request(other),
                }
            }
        }
        let seen = Arc::new(Mutex::new(Vec::new()));
        let backend = Arc::new(RedirectBackend {
            inner: Backend {
                blobs: Default::default(),
                hub: Arc::new(Mutex::new(Default::default())),
                scripts: BTreeMap::new(),
            },
            seen: seen.clone(),
        });
        let boot = bootstrap(
            "const a=await import('./start.js');const b=await import('./start.js');postMessage(a===b&&a.value===42);close()",
            AgentKind::Dedicated,
            ScriptType::Module,
        );
        let mut process =
            AgentThreadProcess::spawn(boot, move |boot| NativeAgentRuntime::create(boot, backend))
                .unwrap();
        verify(message(&mut process), "received===true");
        process.terminate().unwrap();
        assert_eq!(
            *seen.lock().unwrap(),
            vec!["https://a.test/start.js", "https://a.test/final/child.js"]
        );
    }

    #[test]
    fn module_worker_waits_for_top_level_await_and_dynamic_import() {
        let mut scripts = BTreeMap::new();
        scripts.insert(
            "https://a.test/dependency.js".into(),
            "await new Promise(resolve=>setTimeout(resolve,0));export const value=83"
                .encode_utf16()
                .collect(),
        );
        let mut process = launch(
            bootstrap(
                "const m=await import('./dependency.js');postMessage(m.value);close()",
                AgentKind::Dedicated,
                ScriptType::Module,
            ),
            scripts,
        );
        verify(message(&mut process), "String(received===83)");
        process.terminate().unwrap();
    }
    #[test]
    fn cancellation_interrupts_infinite_author_loop_after_publication() {
        let mut process = launch(
            bootstrap(
                "postMessage('entered');while(true){}",
                AgentKind::Dedicated,
                ScriptType::Classic,
            ),
            BTreeMap::new(),
        );
        verify(message(&mut process), "String(received==='entered')");
        let started = Instant::now();
        process.terminate().unwrap();
        assert!(started.elapsed() < Duration::from_secs(5));
    }
    #[test]
    fn audio_worklet_has_native_processor_registry_and_no_worker_or_window_apis() {
        let mut runtime = inline(
            AgentKind::AudioWorklet,
            "registerProcessor('native',class extends AudioWorkletProcessor {process(){return false}});globalThis.allowed=typeof registerProcessor==='function' && typeof AudioWorkletProcessor==='function' && typeof MessagePort==='function' && typeof document==='undefined' && typeof importScripts==='undefined' && typeof close==='undefined' && typeof setTimeout==='undefined' && typeof indexedDB==='undefined' && typeof IDBKeyRange==='undefined' && typeof AbortSignal==='undefined' && typeof URL==='undefined' && typeof Response==='undefined';",
        );
        assert_eq!(
            runtime.host.evaluate_string("String(allowed)").unwrap(),
            "true"
        );
        assert_eq!(
            runtime
                .host
                .evaluate_string(
                    "String(sampleRate===48000 && currentTime===0 && currentFrame===0)"
                )
                .unwrap(),
            "true"
        );
        assert_eq!(runtime.host.evaluate_string("String((()=>{try{registerProcessor('native',class {process(){}});return false}catch(e){return true}})())").unwrap(),"true");
        assert_eq!(runtime.host.evaluate_string("String((()=>{try{new AudioWorkletProcessor();return false}catch(e){return true}})())").unwrap(),"true");
        assert_eq!(runtime.host.evaluate_string("String((()=>{try{registerProcessor('native',class {process(){}});return false}catch(e){return e.name==='NotSupportedError'}})())").unwrap(),"true");
        assert_eq!(runtime.host.evaluate_string("String((()=>{const sentinel={};class P{};Object.defineProperty(P.prototype,'process',{get(){throw sentinel}});try{registerProcessor('throwing',P);return false}catch(e){return e===sentinel}})())").unwrap(),"true");
        assert_eq!(runtime.host.evaluate_string("String((()=>{const f=()=>{};f.prototype={process(){}};try{registerProcessor('arrow',f);return false}catch(e){return e instanceof TypeError}})())").unwrap(),"true");
    }
    #[test]
    fn worklet_module_keeps_creator_eval_policy_and_registration_after_later_module() {
        let backend=Arc::new(Backend{blobs:Default::default(),hub:Arc::new(Mutex::new(Default::default())),scripts:BTreeMap::from([("https://a.test/next.js".into(),"globalThis.second=typeof registerProcessor==='function';registerProcessor('second',class extends AudioWorkletProcessor{process(){return false}})".encode_utf16().collect())])});
        let mut boot = bootstrap(
            "registerProcessor('first',class extends AudioWorkletProcessor {process(){return false}});globalThis.blocked=false;try{eval('1')}catch(e){blocked=true}",
            AgentKind::AudioWorklet,
            ScriptType::Module,
        );
        boot.policy = AgentPolicy {
            url: "https://a.test/document".into(),
            headers: vec![("content-security-policy".into(), "script-src 'self'".into())],
            ..Default::default()
        };
        let mut runtime = NativeAgentRuntime::create(boot, backend).unwrap();
        runtime.start().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while runtime.startup.is_some() {
            runtime.resume_startup().unwrap();
            assert!(
                Instant::now() < deadline,
                "worklet graph preparation deadline"
            );
            std::thread::yield_now();
        }
        assert_eq!(
            runtime.host.evaluate_string("String(blocked)").unwrap(),
            "true"
        );
        runtime
            .dispatch(AgentCommand::Worklet(WorkletCommand::LoadModule {
                id: 7,
                url: "https://a.test/next.js".into(),
                credentials: WorkerCredentials::SameOrigin,
            }))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while runtime.startup.is_some() {
            runtime.resume_startup().unwrap();
            assert!(
                Instant::now() < deadline,
                "worklet graph preparation deadline"
            );
            std::thread::yield_now();
        }
        assert_eq!(runtime.host.evaluate_string("String(second && (()=>{try{registerProcessor('first',class {process(){}});return false}catch(e){return true}})())").unwrap(),"true");
    }
    fn inline(kind: AgentKind, source: &str) -> NativeAgentRuntime {
        let backend = Arc::new(Backend {
            blobs: Default::default(),
            hub: Arc::new(Mutex::new(Default::default())),
            scripts: BTreeMap::new(),
        });
        let mut runtime =
            NativeAgentRuntime::create(bootstrap(source, kind, ScriptType::Classic), backend)
                .unwrap();
        runtime.start().unwrap();
        runtime
    }
    #[test]
    fn service_startup_failure_retains_native_source_position_without_author_error_reads() {
        let mut runtime = inline(
            AgentKind::Service,
            "globalThis.diagnosticReads=0;\nconst thrown={};for(const key of ['name','message','stack','toString'])Object.defineProperty(thrown,key,{get(){diagnosticReads++;throw 99}});\nthrow thrown;",
        );
        let failure = runtime
            .service_startup_failure
            .as_ref()
            .expect("startup failure retained");
        assert_eq!(failure.name, "TypeError");
        assert!(
            failure
                .message
                .contains("worker startup source=\"https://a.test/worker.js\" line=3 column="),
            "{}",
            failure.message
        );
        assert!(failure.message.contains(" offset="));
        assert_eq!(failure.script_failure, None);
        assert_eq!(
            runtime
                .host
                .evaluate_string("String(diagnosticReads)")
                .unwrap(),
            "0"
        );
    }
    #[test]
    fn service_startup_native_metadata_keeps_combined_byte_bound_and_terminal_marker() {
        let mut runtime = inline(AgentKind::Service, "0");
        let realm = runtime.host.active_realm;
        v8::scope!(let handles,&mut runtime.host.isolate);
        let context = v8::Local::new(handles, &runtime.host.contexts[&realm]);
        let scope = &mut v8::ContextScope::new(handles, context);
        v8::tc_scope!(let scope,scope);
        let resource = format!("https://a.test/{}", "界\u{0001}\n".repeat(8192));
        let resource = v8::String::new(scope, &resource).unwrap();
        let origin = v8::ScriptOrigin::new(
            scope,
            resource.into(),
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
        let source = v8::String::new(
            scope,
            "throw Error('original bounded startup failure '+ '界'.repeat(16384))",
        )
        .unwrap();
        let script = v8::Script::compile(scope, source, Some(&origin)).unwrap();
        assert!(script.run(scope).is_none());
        let message = scope.message().expect("native startup Message");
        let failure = service_startup_error(scope, message);
        assert_eq!(failure.name, "TypeError");
        assert_eq!(failure.script_failure, None);
        assert!(
            failure
                .message
                .starts_with("Uncaught Error: original bounded startup failure ")
        );
        assert!(
            failure
                .message
                .contains("worker startup source=\"https://a.test/")
        );
        assert!(failure.message.contains("\\u{1}") && failure.message.contains("\\n"));
        assert!(!failure.message.contains('\u{0001}'));
        assert!(
            failure.message.contains(" line=1 column=") && failure.message.contains(" offset=")
        );
        assert!(failure.message.ends_with(" [truncated]"));
        assert!(
            failure.message.len() < 32 * 1024,
            "combined UTF-8 bytes: {}",
            failure.message.len()
        );
    }
    struct RegistrationGraphBackend {
        base: Backend,
        process: std::sync::atomic::AtomicU64,
    }
    impl WorkerTransport for RegistrationGraphBackend {
        fn request(
            &self,
            request: AgentServiceRequest,
        ) -> Result<AgentServiceResponse, WorkerError> {
            use oryn_js::service_worker_wire::*;
            match request {
                AgentServiceRequest::Service(ServiceRequest::WorkerRegistration) => {
                    Ok(AgentServiceResponse::Service(
                        ServiceResponse::Registration(RegistrationSnapshot {
                            navigation_preload: Default::default(),
                            id: RegistrationId(1),
                            scope: "https://a.test/".into(),
                            update_via_cache: "imports".into(),
                            installing: None,
                            waiting: None,
                            active: Some(WorkerSnapshot {
                                version_id: ServiceVersionId([1; 16]),
                                id: AgentId(self.process.load(Ordering::Acquire)),
                                script_url: "https://a.test/worker.js".into(),
                                state: WorkerState::Activated,
                            }),
                        }),
                    ))
                }
                request => self.base.request(request),
            }
        }
    }
    #[test]
    fn service_wrapper_relationships_trace_identity_but_collect_unreachable_listener_cycles() {
        let backend = Arc::new(RegistrationGraphBackend {
            process: std::sync::atomic::AtomicU64::new(1),
            base: Backend {
                blobs: Default::default(),
                hub: Arc::new(Mutex::new(Default::default())),
                scripts: BTreeMap::new(),
            },
        });
        let mut runtime=NativeAgentRuntime::create(bootstrap("globalThis.saved=registration;globalThis.manager=saved.navigationPreload;globalThis.worker=saved.active;saved.onupdatefound=(()=>{const self=saved;return()=>self})();",AgentKind::Service,ScriptType::Classic),backend.clone()).unwrap();
        runtime.start().unwrap();
        backend.process.store(42, Ordering::Release);
        assert_eq!(
            runtime
                .host
                .evaluate_string("String(worker===registration.active)")
                .unwrap(),
            "true",
            "process restart must preserve logical ServiceWorker wrapper identity"
        );
        let weak = {
            v8::scope!(let handles,&mut runtime.host.isolate);
            let context =
                v8::Local::new(handles, &runtime.host.contexts[&runtime.host.active_realm]);
            let scope = &mut v8::ContextScope::new(handles, context);
            let key = v8::String::new(scope, "saved").unwrap();
            let value = context.global(scope).get(scope, key.into()).unwrap();
            v8::Weak::new(scope, v8::Local::<v8::Object>::try_from(value).unwrap())
        };
        runtime.host.isolate.low_memory_notification();
        assert_eq!(runtime.host.evaluate_string("String(saved===registration&&manager===saved.navigationPreload&&worker===saved.active)").unwrap(),"true");
        runtime.host.execute("saved=manager=worker=null;").unwrap();
        runtime.host.isolate.low_memory_notification();
        runtime.host.isolate.low_memory_notification();
        {
            v8::scope!(let handles,&mut runtime.host.isolate);
            assert!(
                weak.to_local(handles).is_none(),
                "native metadata and listener callback must not root an unreachable registration cycle"
            );
        }
    }
    struct CacheBackend {
        base: Backend,
        store: crate::storage::SharedContextStore,
        lease: crate::storage::cache::CacheLeaseOwner,
    }
    impl WorkerTransport for CacheBackend {
        fn request(
            &self,
            request: AgentServiceRequest,
        ) -> Result<AgentServiceResponse, WorkerError> {
            match request {
                AgentServiceRequest::Service(
                    oryn_js::service_worker_wire::ServiceRequest::Cache { creator, operation },
                ) => {
                    if !matches!(creator, Creator::Agent(AgentId(1))) {
                        return Err(WorkerError::new("SecurityError", "foreign cache creator"));
                    }
                    self.store
                        .cache_operation(
                            self.lease,
                            "https://top.test",
                            "https://a.test",
                            operation,
                        )
                        .map(|result| {
                            AgentServiceResponse::Service(
                                oryn_js::service_worker_wire::ServiceResponse::Cache(result),
                            )
                        })
                        .map_err(agent_error)
                }
                request => self.base.request(request),
            }
        }
    }
    #[test]
    fn native_cache_bindings_consume_real_bodies_and_preserve_deleted_handles() {
        let backend = Arc::new(CacheBackend {
            lease: crate::storage::cache::CacheLeaseOwner::new().unwrap(),
            base: Backend {
                blobs: Default::default(),
                hub: Arc::new(Mutex::new(Default::default())),
                scripts: BTreeMap::new(),
            },
            store: crate::storage::SharedContextStore::memory(Default::default()),
        });
        let source = r#"globalThis.result='pending';(async()=>{
            const cache=await caches.open('\ud800');
            if(await caches.open('\ud800')===cache)throw Error('open must produce a fresh Cache object');
            const body=new Response(new ReadableStream({start(c){c.enqueue(new Uint8Array([104,105]));c.close()}}),{headers:{vary:'Accept-Language'}});
            Object.defineProperty(body,'arrayBuffer',{get(){throw Error('public body accessor')}});
            await cache.put(new Request('https://a.test/data?q=1',{headers:{'Accept-Language':'en'}}),body);
            if(!body.bodyUsed)throw Error('body not consumed');
            const found=await cache.match(new Request('https://a.test/data?q=2#fragment',{headers:{'Accept-Language':'en'}}),{ignoreSearch:true});
            if(await found.text()!=='hi')throw Error('body mismatch');
            if(await cache.match(new Request('https://a.test/data?q=1',{headers:{'Accept-Language':'fr'}})))throw Error('Vary mismatch');
            await caches.delete('\ud800');const replacement=await caches.open('\ud800');
            if((await replacement.keys()).length!==0||(await cache.keys()).length!==1)throw Error('handle lifetime');
            let rejected=false;try{await Cache.prototype.match.call(caches,'https://a.test/data')}catch(e){rejected=e.name==='TypeError'}
            if(!rejected)throw Error('receiver brand');
            if((await caches.keys())[0].charCodeAt(0)!==0xd800)throw Error('DOMString corruption');
            await replacement.put('https://a.test/null',new Response(null));
            await replacement.put('https://a.test/empty',new Response(''));
            const nullBody=await replacement.match('https://a.test/null');
            const emptyBody=await replacement.match('https://a.test/empty');
            if(nullBody.body!==null||emptyBody.body===null)throw Error('null versus empty body lost');
            if(await nullBody.text()!==''||await emptyBody.text()!=='')throw Error('empty body bytes');
            const order=[];const sentinel=new Error('convert');
            const inputs={*[Symbol.iterator](){try{yield {toString(){order.push('convert');throw sentinel}};order.push('advanced')}finally{order.push('closed')}}};
            let conversion;try{await replacement.addAll(inputs)}catch(error){conversion=error}
            if(conversion!==sentinel||order.join(',')!=='convert,closed')throw Error('RequestInfo iterator conversion ordering');
            result='ok';
        })().catch(e=>result='error:'+e.message);"#;
        let mut runtime = NativeAgentRuntime::create(
            bootstrap(source, AgentKind::Dedicated, ScriptType::Classic),
            backend.clone(),
        )
        .unwrap();
        runtime.start().unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            runtime.host.drain_scheduled_timers(false).unwrap();
            // Match the production agent task loop: body consumption and
            // cache promises are native external tasks, not scheduled timers.
            {
                v8::scope!(let handles,&mut runtime.host.isolate);
                let context =
                    v8::Local::new(handles, &runtime.host.contexts[&runtime.host.active_realm]);
                let scope = &mut v8::ContextScope::new(handles, context);
                context
                    .get_slot::<V8BindingState>()
                    .unwrap()
                    .poll_external_ports(scope)
                    .unwrap();
            }
            let result = runtime.host.evaluate_string("result").unwrap();
            if result != "pending" {
                assert_eq!(result, "ok");
                break;
            }
            assert!(Instant::now() < deadline);
        }
        runtime.host.isolate.low_memory_notification();
        runtime.host.isolate.low_memory_notification();
        runtime.host.drain_scheduled_timers(false).unwrap();
        // GC only queues native Cache-handle release. The production agent
        // loop publishes that release at its next external-task checkpoint.
        {
            v8::scope!(let handles,&mut runtime.host.isolate);
            let context =
                v8::Local::new(handles, &runtime.host.contexts[&runtime.host.active_realm]);
            let scope = &mut v8::ContextScope::new(handles, context);
            context
                .get_slot::<V8BindingState>()
                .unwrap()
                .poll_external_ports(scope)
                .unwrap();
        }
        let snapshot = backend
            .store
            .service_storage(crate::storage::ServiceStorageRequest::Read {
                partition: "https://top.test".into(),
                origin: "https://a.test".into(),
                kind: crate::storage::ServiceStorageKind::Caches,
            })
            .unwrap();
        let snapshot: serde_json::Value = serde_json::from_slice(&snapshot.bytes).unwrap();
        assert_eq!(
            snapshot["caches"].as_array().unwrap().len(),
            1,
            "collected Cache objects release deleted cache data"
        );
    }
    #[test]
    fn independent_worker_globals_have_distinct_scope_and_native_platform_bindings() {
        for (kind, name) in [
            (AgentKind::Dedicated, "DedicatedWorkerGlobalScope"),
            (AgentKind::Shared, "SharedWorkerGlobalScope"),
            (AgentKind::Service, "ServiceWorkerGlobalScope"),
        ] {
            let mut runtime = inline(
                kind,
                "globalThis.probe={scope:self.constructor.name,tag:Object.prototype.toString.call(self),worker:self instanceof WorkerGlobalScope,document:typeof document,window:typeof window};",
            );
            let probe: serde_json::Value = serde_json::from_str(
                &runtime
                    .host
                    .evaluate_string("JSON.stringify(probe)")
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(
                probe,
                serde_json::json!({"scope":name,"tag":format!("[object {name}]"),"worker":true,"document":"undefined","window":"undefined"})
            );
            assert_eq!(runtime.host.evaluate_string("String(navigator instanceof WorkerNavigator && Object.prototype.toString.call(navigator)==='[object WorkerNavigator]' && navigator.serviceWorker instanceof ServiceWorkerContainer)").unwrap(),"true");
            if kind == AgentKind::Service {
                assert_eq!(runtime.host.evaluate_string("String(clients instanceof Clients && ['get','matchAll','claim'].every(k=>typeof clients[k]==='function') && (()=>{try{clients.get.call({},'x');return false}catch(e){return e.name==='TypeError'}})())").unwrap(),"true");
            }
            assert_eq!(runtime.host.evaluate_string(r#"(()=>{
                const target=new EventTarget(),event=new Event('probe');let during=0;
                target.addEventListener('probe',e=>during=e.composedPath().length);target.dispatchEvent(event);
                const controller=new AbortController(),signal=AbortSignal.any([controller.signal]);controller.abort();
                const error=new DOMException('missing','NotFoundError');const blob=new Blob(['x']);
                const url=URL.createObjectURL(blob);URL.revokeObjectURL(url);
                return event instanceof Event&&event.target===target&&during===1&&event.composedPath().length===0
                    &&signal.aborted&&signal.reason instanceof DOMException&&error.name==='NotFoundError'&&error.code===8
                    &&new URL('/a','https://example.test/root').href==='https://example.test/a'
                    &&url.startsWith('blob:')&&typeof XMLHttpRequest==='function'&&typeof FileReader==='function';
            })()"#).unwrap(),"true");
        }
    }
    #[test]
    fn insecure_creator_worker_does_not_gain_secure_apis_from_https_script_url() {
        let mut boot = bootstrap(
            "globalThis.probe=typeof caches+'|'+typeof navigator.serviceWorker",
            AgentKind::Dedicated,
            ScriptType::Classic,
        );
        boot.secure_context = false;
        let backend = Arc::new(Backend {
            blobs: Default::default(),
            hub: Arc::new(Mutex::new(Default::default())),
            scripts: BTreeMap::new(),
        });
        let mut runtime = NativeAgentRuntime::create(boot, backend).unwrap();
        runtime.start().unwrap();
        assert_eq!(
            runtime.host.evaluate_string("probe").unwrap(),
            "undefined|undefined"
        );
    }
    #[test]
    fn independent_worker_globals_preserve_native_eventtarget_dispatch() {
        for kind in [AgentKind::Dedicated, AgentKind::Shared, AgentKind::Service] {
            let mut runtime = inline(
                kind,
                r#"
                const order=[];const identity=['addEventListener','removeEventListener','dispatchEvent'].every(name=>self[name]===EventTarget.prototype[name]);
                function removed(){order.push('removed')}
                addEventListener('probe',()=>{order.push('once');removeEventListener('probe',removed);dispatchEvent(new Event('probe'))},{once:true});
                addEventListener('probe',removed);addEventListener('probe',e=>{order.push('listener:'+(e.target===self&&e.currentTarget===self));e.preventDefault()});
                const event=new Event('probe',{cancelable:true});const accepted=dispatchEvent(event);let invalidReceiver=false;
                try{EventTarget.prototype.addEventListener.call({},'probe',()=>{})}catch(e){invalidReceiver=e instanceof TypeError}
                globalThis.probe={identity,order,accepted,canceled:event.defaultPrevented,cleaned:event.currentTarget===null,invalidReceiver};
            "#,
            );
            let probe: serde_json::Value = serde_json::from_str(
                &runtime
                    .host
                    .evaluate_string("JSON.stringify(probe)")
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(
                probe,
                serde_json::json!({"identity":true,"order":["once","listener:true","listener:true"],"accepted":false,"canceled":true,"cleaned":true,"invalidReceiver":true})
            );
        }
    }
    #[test]
    fn shared_worker_connect_is_native_trusted_and_uses_real_port() {
        use oryn_js::message_wire::{MessageHub, MessageRequest, MessageResponse, Principal};
        let mut hub = MessageHub::default();
        let MessageResponse::Channel([client, port]) = hub
            .handle_agent(
                1,
                MessageRequest::CreateChannel,
                "https://a.test",
                "oryn:origin:https://a.test",
            )
            .unwrap()
        else {
            panic!()
        };
        hub.set_partition(Principal::Agent(1), "context/site".into());
        let backend = Arc::new(Backend {
            blobs: Default::default(),
            hub: Arc::new(Mutex::new(hub)),
            scripts: BTreeMap::new(),
        });
        let mut runtime=NativeAgentRuntime::create(bootstrap("globalThis.connected=false;onconnect=e=>{connected=e.isTrusted&&e.source===e.ports[0]&&e.ports.length===1&&e.ports[0] instanceof MessagePort&&e.data==='';e.ports[0].postMessage(new Map([['connected',connected]]))}",AgentKind::Shared,ScriptType::Classic),backend.clone()).unwrap();
        runtime.start().unwrap();
        runtime
            .dispatch(AgentCommand::Connect {
                client: AgentClientId {
                    id: 1,
                    secret: [0; 32],
                },
                port,
            })
            .unwrap();
        assert_eq!(
            runtime.host.evaluate_string("String(connected)").unwrap(),
            "true"
        );
        let AgentServiceResponse::Message(MessageResponse::PortMessages { mut messages, .. }) =
            backend
                .request(AgentServiceRequest::Message(MessageRequest::PollPort {
                    port: client,
                    max: 1,
                }))
                .unwrap()
        else {
            panic!()
        };
        assert_eq!(messages.len(), 1);
        verify(
            messages.remove(0),
            "String(received instanceof Map&&received.get('connected')===true)",
        );
    }
    fn service_poll(runtime: &mut NativeAgentRuntime) -> Vec<AgentEvent> {
        v8::scope!(let handles,&mut runtime.host.isolate);
        let context = v8::Local::new(handles, &runtime.host.contexts[&runtime.host.active_realm]);
        let scope = &mut v8::ContextScope::new(handles, context);
        let state = context.get_slot::<V8BindingState>().unwrap();
        scope.perform_microtask_checkpoint();
        oryn_js::v8_bindings::service_worker_events::poll(scope, &state).unwrap()
    }
    fn fetch_request(id: u64) -> AgentFetchRequest {
        AgentFetchRequest {
            keepalive: false,
            privileged_range: false,
            cache: Default::default(),
            navigation: Default::default(),
            integrity: String::new(),
            navigation_preload: false,
            streaming: false,
            id,
            client_id: "client-1".into(),
            url: "https://a.test/resource".into(),
            method: "GET".into(),
            headers: vec![("x-input".into(), "present".into())],
            body: Vec::new(),
            destination: "document".into(),
            mode: "navigate".into(),
            credentials: "include".into(),
            redirect: "follow".into(),
        }
    }
    #[test]
    fn service_lifecycle_waits_for_nested_extensions_and_rejects_late_waituntil() {
        let mut runtime = inline(
            AgentKind::Service,
            r#"globalThis.valid=false;oninstall=e=>{globalThis.held=e;valid=e.isTrusted&&e instanceof ExtendableEvent&&e.target===self;e.waitUntil(Promise.resolve().then(()=>e.waitUntil(new Promise(resolve=>globalThis.finish=resolve))))}"#,
        );
        runtime
            .dispatch(AgentCommand::Lifecycle {
                id: 41,
                event: LifecycleEvent::Install,
                payload: None,
            })
            .unwrap();
        assert_eq!(
            runtime.host.evaluate_string("String(valid)").unwrap(),
            "true"
        );
        assert!(service_poll(&mut runtime).is_empty());
        runtime.host.execute("finish()").unwrap();
        assert!(matches!(
            service_poll(&mut runtime).as_slice(),
            [AgentEvent::LifecycleFinished {
                id: 41,
                result: Ok(())
            }]
        ));
        assert_eq!(runtime.host.evaluate_string("(()=>{try{held.waitUntil(Promise.resolve())}catch(e){return e.name==='InvalidStateError'}return false})()").unwrap(),"true");
        runtime
            .host
            .execute("onactivate=e=>e.waitUntil(Promise.reject(new Error('install failed')))")
            .unwrap();
        runtime
            .dispatch(AgentCommand::Lifecycle {
                id: 42,
                event: LifecycleEvent::Activate,
                payload: None,
            })
            .unwrap();
        assert!(matches!(
            service_poll(&mut runtime).as_slice(),
            [AgentEvent::LifecycleFinished {
                id: 42,
                result: Err(_)
            }]
        ));
    }
    #[test]
    fn service_fetch_consumes_native_response_and_retains_waituntil_after_response() {
        let mut runtime = inline(
            AgentKind::Service,
            r#"
            globalThis.valid=false;onfetch=e=>{
                valid=e.isTrusted&&e instanceof FetchEvent&&e.request instanceof Request&&e.request.url==='https://a.test/resource'&&e.clientId==='client-1'&&e.request.headers.get('x-input')==='present';
                const response=new Response(new ReadableStream({start(c){c.enqueue(new Uint8Array([7,8,9]));c.close()}}),{status:201,headers:{'x-output':'native'}});
                for(const name of ['arrayBuffer','text','json','headers','status'])Object.defineProperty(response,name,{get(){throw Error('author response getter')}});
                e.respondWith(Promise.resolve(response));e.waitUntil(new Promise(resolve=>globalThis.finish=resolve));
                try{e.respondWith(new Response('second'))}catch(error){valid=valid&&error.name==='InvalidStateError'}
            };
        "#,
        );
        runtime
            .dispatch(AgentCommand::Fetch(fetch_request(55)))
            .unwrap();
        assert_eq!(
            runtime.host.evaluate_string("String(valid)").unwrap(),
            "true"
        );
        let mut events = service_poll(&mut runtime);
        events.extend(service_poll(&mut runtime));
        assert_eq!(events.len(), 1);
        let AgentEvent::FetchResponse {
            id: 55,
            response: Ok(response),
        } = &events[0]
        else {
            panic!("missing service response")
        };
        assert_eq!(response.status, 201);
        assert_eq!(response.body, [7, 8, 9]);
        assert!(
            response
                .headers
                .contains(&("x-output".into(), "native".into()))
        );
        runtime.host.execute("finish()").unwrap();
        assert!(matches!(
            service_poll(&mut runtime).as_slice(),
            [AgentEvent::LifecycleFinished {
                id: 55,
                result: Ok(())
            }]
        ));
    }
    #[test]
    fn service_stream_emits_headers_before_eof_and_obeys_native_demand() {
        let mut runtime = inline(
            AgentKind::Service,
            r#"onfetch=e=>{const response=new Response(new ReadableStream({start(c){c.enqueue(new Uint8Array([1,2]));globalThis.finishBody=()=>c.close();},cancel(){globalThis.cancelled=true;}}));Object.defineProperty(response,'body',{get(){throw Error('author body getter')}});e.respondWith(response);e.waitUntil(new Promise(resolve=>globalThis.finishLifetime=resolve));}"#,
        );
        let mut request = fetch_request(82);
        request.streaming = true;
        runtime.dispatch(AgentCommand::Fetch(request)).unwrap();
        assert!(matches!(
            service_poll(&mut runtime).as_slice(),
            [AgentEvent::FetchResponseStart { id: 82, .. }]
        ));
        for _ in 0..3 {
            assert!(service_poll(&mut runtime).is_empty());
        }
        runtime
            .dispatch(AgentCommand::FetchPull { id: 82 })
            .unwrap();
        let mut events = Vec::new();
        for _ in 0..4 {
            events.extend(service_poll(&mut runtime));
        }
        assert!(
            matches!(events.as_slice(),[AgentEvent::FetchResponseChunk{id:82,bytes}]if bytes==&vec![1,2])
        );
        runtime.host.execute("finishBody()").unwrap();
        assert!(service_poll(&mut runtime).is_empty());
        runtime
            .dispatch(AgentCommand::FetchPull { id: 82 })
            .unwrap();
        let mut events = Vec::new();
        for _ in 0..4 {
            events.extend(service_poll(&mut runtime));
        }
        assert!(matches!(
            events.as_slice(),
            [AgentEvent::FetchResponseEnd {
                id: 82,
                result: Ok(())
            }]
        ));
        runtime.host.execute("finishLifetime()").unwrap();
        assert!(matches!(
            service_poll(&mut runtime).as_slice(),
            [AgentEvent::LifecycleFinished {
                id: 82,
                result: Ok(())
            }]
        ));
        runtime.host.execute("globalThis.invalidCancelled=false;onfetch=e=>e.respondWith(new Response(new ReadableStream({start(c){c.enqueue('not bytes')},cancel(){invalidCancelled=true}})))").unwrap();
        let mut request = fetch_request(83);
        request.streaming = true;
        runtime.dispatch(AgentCommand::Fetch(request)).unwrap();
        assert!(matches!(
            service_poll(&mut runtime).as_slice(),
            [AgentEvent::FetchResponseStart { id: 83, .. }]
        ));
        runtime
            .dispatch(AgentCommand::FetchPull { id: 83 })
            .unwrap();
        let mut events = Vec::new();
        for _ in 0..4 {
            events.extend(service_poll(&mut runtime));
        }
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::FetchResponseEnd {
                id: 83,
                result: Err(_)
            }
        )));
        assert_eq!(
            runtime
                .host
                .evaluate_string("String(invalidCancelled)")
                .unwrap(),
            "true"
        );
    }

    #[test]
    fn service_fetch_fallback_and_unusable_responses_are_distinct() {
        let mut runtime = inline(AgentKind::Service, "onfetch=e=>{globalThis.held=e}");
        runtime
            .dispatch(AgentCommand::Fetch(fetch_request(70)))
            .unwrap();
        let events = service_poll(&mut runtime);
        assert!(
            events
                .iter()
                .any(|event| matches!(event, AgentEvent::FetchFallback { id: 70 }))
        );
        assert_eq!(runtime.host.evaluate_string("(()=>{try{held.respondWith(new Response('late'))}catch(e){return e.name==='InvalidStateError'}return false})()").unwrap(),"true");
        for (id, source) in [
            (
                71,
                "onfetch=e=>e.respondWith(new Proxy(new Response('x'),{}))",
            ),
            (
                72,
                "const used=new Response('x');used.text();onfetch=e=>e.respondWith(used)",
            ),
            (
                73,
                "onfetch=e=>e.respondWith(Promise.reject(new Error('fetch rejected')))",
            ),
        ] {
            runtime.host.execute(source).unwrap();
            runtime
                .dispatch(AgentCommand::Fetch(fetch_request(id)))
                .unwrap();
            let mut events = service_poll(&mut runtime);
            events.extend(service_poll(&mut runtime));
            assert!(events.iter().any(|event|matches!(event,AgentEvent::FetchResponse{id:actual,response:Err(_)}if *actual==id)));
            if id == 73 {
                assert!(events.iter().any(|event|matches!(event,AgentEvent::FetchResponse{id:73,response:Err(error)}if error.script_failure==Some(ScriptFailureKind::Fetch))));
            }
            assert!(
                !events
                    .iter()
                    .any(|event| matches!(event, AgentEvent::FetchFallback { .. }))
            );
        }
    }
    #[test]
    fn service_task_cancellation_aborts_only_owned_stream_signal_and_lifetime() {
        let mut runtime = inline(
            AgentKind::Service,
            r#"
            globalThis.cancelled=false;
            onfetch=e=>{globalThis.request=e.request;globalThis.held=e;
                e.respondWith(new Response(new ReadableStream({start(controller){globalThis.controller=controller},cancel(reason){cancelled=reason.name==='AbortError'}})));
                e.waitUntil(new Promise(resolve=>globalThis.finishFetch=resolve));};
            oninstall=e=>e.waitUntil(new Promise(resolve=>globalThis.finishInstall=resolve));
        "#,
        );
        runtime
            .dispatch(AgentCommand::Fetch(fetch_request(90)))
            .unwrap();
        runtime
            .dispatch(AgentCommand::Lifecycle {
                id: 91,
                event: LifecycleEvent::Install,
                payload: None,
            })
            .unwrap();
        assert!(service_poll(&mut runtime).is_empty());
        let cancelled = {
            v8::scope!(let handles,&mut runtime.host.isolate);
            let context =
                v8::Local::new(handles, &runtime.host.contexts[&runtime.host.active_realm]);
            let scope = &mut v8::ContextScope::new(handles, context);
            let state = context.get_slot::<V8BindingState>().unwrap();
            oryn_js::v8_bindings::service_worker_events::cancel(scope, &state, 90).unwrap()
        };
        assert!(cancelled.iter().any(|event|matches!(event,AgentEvent::FetchResponse{id:90,response:Err(error)}if error.script_failure==Some(ScriptFailureKind::Cancelled))));
        assert!(cancelled.iter().any(|event| matches!(
            event,
            AgentEvent::LifecycleFinished {
                id: 90,
                result: Err(_)
            }
        )));
        assert_eq!(runtime.host.evaluate_string("String(cancelled&&request.signal.aborted&&request.signal.reason.name==='AbortError')").unwrap(),"true");
        runtime.host.execute("finishFetch()").unwrap();
        assert!(service_poll(&mut runtime).is_empty());
        runtime.host.execute("finishInstall()").unwrap();
        assert!(matches!(
            service_poll(&mut runtime).as_slice(),
            [AgentEvent::LifecycleFinished {
                id: 91,
                result: Ok(())
            }]
        ));
    }
    #[test]
    fn service_response_body_limit_cancels_the_native_stream() {
        let mut runtime = inline(
            AgentKind::Service,
            r#"
            globalThis.cancelled=false;onfetch=e=>e.respondWith(new Response(new ReadableStream({
                start(c){c.enqueue(new Uint8Array(33*1024*1024))},cancel(){cancelled=true}
            })));
        "#,
        );
        runtime
            .dispatch(AgentCommand::Fetch(fetch_request(99)))
            .unwrap();
        let mut events = service_poll(&mut runtime);
        events.extend(service_poll(&mut runtime));
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::FetchResponse {
                id: 99,
                response: Err(_)
            }
        )));
        assert_eq!(
            runtime.host.evaluate_string("String(cancelled)").unwrap(),
            "true"
        );
    }
    #[derive(Default)]
    struct ConstructorProbe {
        secure_publications: Mutex<Vec<(Creator, bool)>>,
        creations: Mutex<Vec<AgentCreation>>,
        messages: Mutex<Vec<oryn_js::message_wire::WireValue>>,
        terminations: Mutex<usize>,
        hub: Mutex<oryn_js::message_wire::MessageHub>,
        registrations: Mutex<Vec<(u64, u64)>>,
    }
    impl WorkerTransport for ConstructorProbe {
        fn request(
            &self,
            request: AgentServiceRequest,
        ) -> Result<AgentServiceResponse, WorkerError> {
            match request {
                AgentServiceRequest::PublishSecureContext { creator, secure } => {
                    let Creator::Window {
                        target: oryn_js::message_wire::WindowTarget::Root,
                        document_generation,
                        stream_epoch,
                    } = &creator
                    else {
                        return Err(WorkerError::new(
                            "SecurityError",
                            "fixture requires root creator",
                        ));
                    };
                    self.hub
                        .lock()
                        .unwrap()
                        .root_document_authority(1, *document_generation, *stream_epoch)
                        .map_err(|message| WorkerError::new("InvalidStateError", message))?;
                    self.secure_publications
                        .lock()
                        .unwrap()
                        .push((creator, secure));
                    Ok(AgentServiceResponse::Done)
                }
                AgentServiceRequest::Create(creation) => {
                    if !self
                        .secure_publications
                        .lock()
                        .unwrap()
                        .iter()
                        .any(|(creator, _)| creator == &creation.creator)
                    {
                        return Err(WorkerError::new(
                            "SecurityError",
                            "fixture worker lacks secure-context publication",
                        ));
                    }
                    self.creations.lock().unwrap().push(creation);
                    Ok(AgentServiceResponse::Connected(AgentConnection {
                        agent: AgentId(1),
                        client: AgentClientId {
                            id: 1,
                            secret: [17; 32],
                        },
                        port: None,
                    }))
                }
                AgentServiceRequest::Post { value, .. } => {
                    self.messages.lock().unwrap().push(value);
                    Ok(AgentServiceResponse::Done)
                }
                AgentServiceRequest::Poll { .. } => Ok(AgentServiceResponse::Events(
                    self.messages
                        .lock()
                        .unwrap()
                        .drain(..)
                        .map(|value| AgentEvent::Message {
                            client: None,
                            value,
                        })
                        .collect(),
                )),
                AgentServiceRequest::Terminate { .. } => {
                    *self.terminations.lock().unwrap() += 1;
                    Ok(AgentServiceResponse::Done)
                }
                AgentServiceRequest::Release { .. } => Ok(AgentServiceResponse::Done),
                AgentServiceRequest::Message(request) => {
                    let registration = match &request {
                        oryn_js::message_wire::MessageRequest::RegisterRootDocument {
                            document_generation,
                            stream_epoch,
                        } => Some((*document_generation, *stream_epoch)),
                        _ => None,
                    };
                    let response = self
                        .hub
                        .lock()
                        .unwrap()
                        .handle(1, request, &BTreeMap::from([(1, "https://a.test".into())]))
                        .map_err(agent_error)?;
                    if let Some(registration) = registration {
                        self.registrations.lock().unwrap().push(registration);
                    }
                    Ok(AgentServiceResponse::Message(response))
                }
                _ => Err(agent_error("unexpected constructor probe request")),
            }
        }
    }
    #[test]
    fn native_worker_constructor_converts_once_and_posts_intrinsic_clones() {
        let backend = Arc::new(ConstructorProbe::default());
        let mut host = RawV8Host::new();
        host.install_dom(
            Rc::new(RefCell::new(
                oryn_dom::html::parse("<body>").into_runtime().unwrap(),
            )),
            1,
        )
        .unwrap();
        host.binding_states[&host.active_realm]
            .set_message_transport(Arc::new(AgentMessages(backend.clone())));
        host.binding_states[&host.active_realm].set_worker_transport(
            backend.clone(),
            None,
            Some("https://a.test/directory/page".into()),
        );
        host.execute(r#"
            globalThis.order=[];globalThis.worker=new Worker('../worker.js',{
                get credentials(){order.push('credentials');return 'include'},get name(){order.push('name');return '\ud800'},get type(){order.push('type');return 'module'}});
            globalThis.valid=false;worker.onmessage=e=>{valid=e.isTrusted&&e.target===worker&&e.data.self===e.data&&e.data.map instanceof Map&&e.data.map.get(e.data)===83n};
            const d={};d.self=d;d.map=new Map([[d,83n]]);JSON.stringify=JSON.parse=()=>{throw Error('author JSON')};
            worker.postMessage(d);
        "#).unwrap();
        assert_eq!(
            host.evaluate_string("order.join(',')").unwrap(),
            "credentials,name,type"
        );
        {
            let creations = backend.creations.lock().unwrap();
            assert_eq!(creations.len(), 1);
            assert_eq!(creations[0].url, "https://a.test/worker.js");
            assert_eq!(creations[0].options.name, [0xd800]);
            assert_eq!(creations[0].options.credentials, WorkerCredentials::Include);
            assert_eq!(creations[0].options.script_type, ScriptType::Module);
            assert!(matches!(&creations[0].creator, Creator::Window { .. }));
            let Creator::Window {
                target: oryn_js::message_wire::WindowTarget::Root,
                document_generation,
                stream_epoch,
            } = &creations[0].creator
            else {
                panic!("root creator required")
            };
            let registrations = backend.registrations.lock().unwrap();
            assert!(!registrations.is_empty());
            assert!(
                registrations
                    .iter()
                    .all(|identity| identity == &(*document_generation, *stream_epoch))
            );
            let publications = backend.secure_publications.lock().unwrap();
            #[cfg(unix)]
            let expected_publications = vec![(creations[0].creator.clone(), false); 2];
            #[cfg(not(unix))]
            let expected_publications = vec![(creations[0].creator.clone(), false)];
            assert_eq!(
                *publications, expected_publications,
                "Worker admission and native shared-memory binding each publish the same actual insecure creator before creation"
            );
        }
        {
            v8::scope!(let handles,&mut host.isolate);
            let context = v8::Local::new(handles, &host.contexts[&host.active_realm]);
            let scope = &mut v8::ContextScope::new(handles, context);
            context
                .get_slot::<V8BindingState>()
                .unwrap()
                .poll_external_ports(scope)
                .unwrap();
        }
        assert_eq!(host.evaluate_string("String(valid)").unwrap(), "true");
        assert_eq!(host.evaluate_string("(()=>{try{worker.postMessage(new Proxy({},{}))}catch(e){return e.name==='DataCloneError'}return false})()").unwrap(),"true");
        assert!(backend.messages.lock().unwrap().is_empty());
        host.execute("worker.terminate();worker.terminate()")
            .unwrap();
        assert_eq!(*backend.terminations.lock().unwrap(), 1);
    }
    #[test]
    fn constructor_fixture_message_service_requires_registered_document() {
        use oryn_js::message_wire::{MessageRequest, MessageResponse, WindowTarget};
        let backend = ConstructorProbe::default();
        let poll = |generation| {
            AgentServiceRequest::Message(MessageRequest::InDocument {
                target: WindowTarget::Root,
                document_generation: generation,
                stream_epoch: 3,
                request: Box::new(MessageRequest::PollWindow {
                    target: WindowTarget::Root,
                    max: 1,
                }),
            })
        };
        assert!(backend.request(poll(7)).is_err());
        let publication = |generation| AgentServiceRequest::PublishSecureContext {
            creator: Creator::Window {
                target: WindowTarget::Root,
                document_generation: generation,
                stream_epoch: 3,
            },
            secure: false,
        };
        assert!(backend.request(publication(7)).is_err());
        assert!(matches!(
            backend
                .request(AgentServiceRequest::Message(
                    MessageRequest::RegisterRootDocument {
                        document_generation: 7,
                        stream_epoch: 3
                    }
                ))
                .unwrap(),
            AgentServiceResponse::Message(MessageResponse::Done)
        ));
        assert!(matches!(
            backend.request(publication(7)).unwrap(),
            AgentServiceResponse::Done
        ));
        assert!(backend.request(publication(8)).is_err());
        assert!(
            matches!(backend.request(poll(7)).unwrap(),AgentServiceResponse::Message(MessageResponse::WindowMessages(messages)) if messages.is_empty())
        );
        assert!(backend.request(poll(8)).is_err());
        backend
            .request(AgentServiceRequest::Message(
                MessageRequest::RetireRootDocument {
                    document_generation: 7,
                    stream_epoch: 3,
                },
            ))
            .unwrap();
        assert!(backend.request(publication(7)).is_err());
        assert!(backend.request(poll(7)).is_err());
        assert!(
            backend
                .request(AgentServiceRequest::Message(
                    MessageRequest::RegisterRootDocument {
                        document_generation: 7,
                        stream_epoch: 3
                    }
                ))
                .is_err()
        );
        assert_eq!(*backend.registrations.lock().unwrap(), vec![(7, 3)]);
    }
    #[derive(Default)]
    struct PlatformProbe {
        requests: Mutex<Vec<oryn_js::platform::PlatformHostRequest>>,
        ready: Mutex<std::collections::VecDeque<oryn_js::platform::PlatformHostCompletion>>,
        csp_polls: Mutex<Vec<u64>>,
    }
    impl WorkerTransport for PlatformProbe {
        fn request(
            &self,
            request: AgentServiceRequest,
        ) -> Result<AgentServiceResponse, WorkerError> {
            use oryn_js::platform::*;
            match request {
                // This fixture emits no CSP reports. Its worker still polls
                // the independent event channel at every checkpoint.
                AgentServiceRequest::CspEvents { after } => {
                    self.csp_polls.lock().unwrap().push(after);
                    Ok(AgentServiceResponse::CspEvents(Vec::new()))
                }
                AgentServiceRequest::PlatformSubmit {
                    operation_id,
                    request,
                } => {
                    self.requests.lock().unwrap().push(request.clone());
                    let result = match request {
                        PlatformHostRequest::HttpStart { request } => {
                            Ok(PlatformHostResult::HttpStarted {
                                timing: None,
                                internal_response: None,
                                response_type: None,
                                upload: None,
                                cors_filtered: false,
                                redirected: false,
                                stream_id: 1,
                                status: 200,
                                url: request.url,
                                headers: vec![("content-type".into(), "text/plain".into())],
                            })
                        }
                        PlatformHostRequest::HttpRead { stream_id, .. } => {
                            Ok(PlatformHostResult::HttpChunk {
                                encoded_body_size: None,
                                stream_id,
                                bytes: b"agent-network".to_vec(),
                                done: true,
                            })
                        }
                        PlatformHostRequest::HttpAbort { .. } => {
                            Ok(PlatformHostResult::HttpAborted)
                        }
                        PlatformHostRequest::IndexedDb {
                            command: IdbHostCommand::ListDatabases,
                            ..
                        } => Ok(PlatformHostResult::IndexedDb {
                            response: IdbHostResponse {
                                databases: vec![IdbDatabaseInfo {
                                    name: "agent-storage".into(),
                                    version: 2,
                                }],
                                ..Default::default()
                            },
                        }),
                        _ => Err("unexpected agent platform operation".into()),
                    };
                    self.ready
                        .lock()
                        .unwrap()
                        .push_back(PlatformHostCompletion {
                            operation_id,
                            result,
                        });
                    Ok(AgentServiceResponse::Done)
                }
                AgentServiceRequest::PlatformPoll { max } => {
                    let mut ready = self.ready.lock().unwrap();
                    let count = max.min(ready.len());
                    Ok(AgentServiceResponse::PlatformCompletions(
                        ready.drain(..count).collect(),
                    ))
                }
                AgentServiceRequest::PlatformRetire
                | AgentServiceRequest::PlatformReleaseResult { .. }
                | AgentServiceRequest::PlatformCancel { .. } => Ok(AgentServiceResponse::Done),
                _ => Err(agent_error("unexpected worker probe service request")),
            }
        }
    }
    #[test]
    fn independent_worker_fetch_and_indexeddb_use_agent_platform_correlations() {
        use oryn_js::platform::*;
        let backend = Arc::new(PlatformProbe::default());
        let runtime_backend = backend.clone();
        let bootstrap = bootstrap(
            "Promise.all([fetch('./api',{credentials:'include'}).then(r=>r.text()),indexedDB.databases()]).then(([text,databases])=>{postMessage({text,databases});close()})",
            AgentKind::Dedicated,
            ScriptType::Classic,
        );
        let mut process = AgentThreadProcess::spawn(bootstrap, move |bootstrap| {
            NativeAgentRuntime::create(bootstrap, runtime_backend)
        })
        .unwrap();
        verify(
            message(&mut process),
            "String(received.text==='agent-network'&&received.databases[0].name==='agent-storage'&&received.databases[0].version===2)",
        );
        process.terminate().unwrap();
        let polls = backend.csp_polls.lock().unwrap();
        assert!(!polls.is_empty());
        assert!(polls.iter().all(|after| *after == 0));
        drop(polls);
        let requests = backend.requests.lock().unwrap();
        assert!(requests.iter().any(|request|matches!(request,PlatformHostRequest::HttpStart{request}if request.url=="https://a.test/api"&&request.credentials=="include"&&request.initiator_url.as_deref()==Some("https://a.test/worker.js"))));
        assert!(requests.iter().any(|request|matches!(request,PlatformHostRequest::IndexedDb{origin,..}if origin=="https://a.test")));
    }
    #[test]
    fn platform_fixture_csp_polling_preserves_cursor_and_response_type() {
        let backend = PlatformProbe::default();
        for after in [0, 17, u64::MAX] {
            assert!(
                matches!(backend.request(AgentServiceRequest::CspEvents{after}).unwrap(),AgentServiceResponse::CspEvents(events) if events.is_empty())
            );
        }
        assert_eq!(*backend.csp_polls.lock().unwrap(), vec![0, 17, u64::MAX]);
        assert!(backend.requests.lock().unwrap().is_empty());
        assert!(backend.ready.lock().unwrap().is_empty());
        assert!(
            matches!(backend.request(AgentServiceRequest::PlatformPoll{max:1}).unwrap(),AgentServiceResponse::PlatformCompletions(events) if events.is_empty())
        );
        assert!(
            backend
                .request(AgentServiceRequest::Message(
                    oryn_js::message_wire::MessageRequest::CreateChannel
                ))
                .is_err()
        );
    }
    #[cfg(unix)]
    #[test]
    fn worker_import_waits_off_thread_and_preserves_response_policy() {
        struct HeldImport {
            fallback: Backend,
            entered: std::sync::mpsc::Sender<()>,
            release: Mutex<std::sync::mpsc::Receiver<()>>,
        }
        impl WorkerTransport for HeldImport {
            fn request(
                &self,
                request: AgentServiceRequest,
            ) -> Result<AgentServiceResponse, WorkerError> {
                if let AgentServiceRequest::ImportModule {
                    url,
                    kind,
                    referrer_source,
                    referrer_policy,
                } = request
                {
                    assert_eq!(url, "https://a.test/held.js");
                    assert_eq!(kind, oryn_js::worker_wire::ImportedModuleType::JavaScript);
                    assert_eq!(referrer_source, "https://a.test/worker.js");
                    assert_eq!(referrer_policy, "no-referrer");
                    self.entered.send(()).unwrap();
                    self.release.lock().unwrap().recv().unwrap();
                    Ok(AgentServiceResponse::Script {
                        url: "https://a.test/final.js".into(),
                        source: "export const url=import.meta.url".encode_utf16().collect(),
                        referrer_policy: "same-origin".into(),
                    })
                } else {
                    self.fallback.request(request)
                }
            }
        }
        let (entered, observed) = std::sync::mpsc::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        let backend = Arc::new(HeldImport {
            fallback: Backend {
                blobs: Default::default(),
                hub: Default::default(),
                scripts: Default::default(),
            },
            entered,
            release: Mutex::new(blocked),
        });
        let mut boot = bootstrap(
            "globalThis.done=false;import('./held.js').then(m=>{globalThis.imported=m.url;done=true})",
            AgentKind::Dedicated,
            ScriptType::Classic,
        );
        boot.referrer_policy = "no-referrer".into();
        let mut runtime = NativeAgentRuntime::create(boot, backend).unwrap();
        runtime.start().unwrap();
        assert!(observed.try_recv().is_err());
        runtime.host.drain_async_fetches(false).unwrap();
        observed.recv_timeout(Duration::from_secs(5)).unwrap();
        runtime.host.execute("globalThis.independent=42").unwrap();
        assert_eq!(
            runtime
                .host
                .evaluate_string("independent+'|'+done")
                .unwrap(),
            "42|false"
        );
        release.send(()).unwrap();
        dynamic_import_continuation_tests::settle(&mut runtime.host);
        assert_eq!(
            runtime.host.evaluate_string("imported+'|'+done").unwrap(),
            "https://a.test/final.js|true"
        );
    }
    #[cfg(unix)]
    #[test]
    fn worker_json_dynamic_import_uses_typed_graph_and_service_rejects_before_io() {
        let backend = Arc::new(Backend {
            blobs: Default::default(),
            hub: Default::default(),
            scripts: BTreeMap::from([(
                "https://a.test/value.json".into(),
                "{\"value\":42}".encode_utf16().collect(),
            )]),
        });
        let mut runtime=NativeAgentRuntime::create(bootstrap("import('./value.json',{with:{type:'json'}}).then(m=>globalThis.answer=m.default.value)",AgentKind::Dedicated,ScriptType::Classic),backend.clone()).unwrap();
        runtime.start().unwrap();
        dynamic_import_continuation_tests::settle(&mut runtime.host);
        assert_eq!(
            runtime.host.evaluate_string("String(answer)").unwrap(),
            "42"
        );
        let mut service = NativeAgentRuntime::create(
            bootstrap(
                "import('./value.json',{with:{type:'json'}}).catch(e=>globalThis.failure=e.name)",
                AgentKind::Service,
                ScriptType::Classic,
            ),
            backend,
        )
        .unwrap();
        service.start().unwrap();
        assert!(!service.host.has_pending_dynamic_imports(false));
        assert_eq!(
            service.host.evaluate_string("failure").unwrap(),
            "TypeError"
        );
    }

    #[cfg(unix)]
    #[test]
    fn worker_text_import_retains_native_settlement_after_observer_failure() {
        let backend = Arc::new(Backend {
            blobs: Default::default(),
            hub: Default::default(),
            scripts: BTreeMap::from([(
                "https://a.test/value.txt".into(),
                "text value".encode_utf16().collect(),
            )]),
        });
        let mut runtime=NativeAgentRuntime::create(bootstrap("globalThis.done=false;import('./value.txt',{with:{type:'text'}}).then(m=>{globalThis.answer=m.default;done=true})",AgentKind::Dedicated,ScriptType::Classic),backend).unwrap();
        runtime.start().unwrap();
        FAIL_MODULE_SETTLEMENT.with(|fail| fail.set(true));
        let limit = Instant::now() + Duration::from_secs(5);
        let jobs = loop {
            runtime.host.drain_async_fetches(false).unwrap();
            if let Some(jobs) = runtime
                .host
                .isolate
                .get_slot::<Rc<RefCell<ModuleSettlements>>>()
                .cloned()
                && jobs.borrow().jobs.values().any(|job| job.outcome.is_some())
            {
                break jobs;
            }
            assert!(Instant::now() < limit);
            std::thread::yield_now();
        };
        assert_eq!(
            runtime.host.evaluate_string("String(done)").unwrap(),
            "false"
        );
        for job in jobs.borrow_mut().jobs.values_mut() {
            job.retry = Instant::now();
        }
        runtime.host.poll_module_settlements().unwrap();
        assert_eq!(
            runtime.host.evaluate_string("done+'|'+answer").unwrap(),
            "true|text value"
        );
        assert!(jobs.borrow().jobs.is_empty());
    }

    #[test]
    fn worker_typed_startup_keeps_graph_on_native_preparation_failure() {
        let backend = Arc::new(Backend {
            blobs: Default::default(),
            hub: Default::default(),
            scripts: BTreeMap::from([(
                "https://a.test/value.json".into(),
                "{\"value\":42}".encode_utf16().collect(),
            )]),
        });
        let mut runtime=NativeAgentRuntime::create(bootstrap("import value from './value.json' with {type:'json'};globalThis.answer=value.value;",AgentKind::Dedicated,ScriptType::Module),backend).unwrap();
        FAIL_TYPED_PREPARATION.with(|failure| {
            *failure.borrow_mut() = Some(
                crate::script_load_failure::ScriptLoadFailure::Infrastructure(
                    "injected startup preparation".into(),
                ),
            )
        });
        runtime.start().unwrap();
        let limit = Instant::now() + Duration::from_secs(5);
        while !runtime
            .startup
            .as_ref()
            .is_some_and(|pending| pending.prepared && pending.retry.is_some())
        {
            runtime.resume_startup().unwrap();
            assert!(Instant::now() < limit);
            std::thread::yield_now();
        }
        assert_eq!(
            runtime.host.evaluate_string("typeof answer").unwrap(),
            "undefined"
        );
        runtime.startup.as_mut().unwrap().retry = None;
        runtime.resume_startup().unwrap();
        assert!(runtime.startup.is_none());
        assert_eq!(
            runtime.host.evaluate_string("String(answer)").unwrap(),
            "42"
        );
    }
}
