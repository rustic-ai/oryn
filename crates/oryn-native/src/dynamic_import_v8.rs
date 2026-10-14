// Included in raw_v8. Import callbacks capture exact native ownership and
// return immediately; IO and graph discovery resume on later host turns.
use crate::executable::module_graph_load::{
    ModuleGraphBackend, ModuleGraphLoad, SharedFetchRegistry,
};
#[derive(Default)]
struct DynamicImports {
    pending: std::collections::VecDeque<PendingDynamicImport>,
    direct: std::collections::VecDeque<PendingDirectModule>,
    admitted: Rc<std::cell::Cell<usize>>,
    bytes: Arc<std::sync::atomic::AtomicUsize>,
    slots: Arc<std::sync::atomic::AtomicUsize>,
    fetches: SharedFetchRegistry,
}
struct DynamicImportLimits {
    bytes: Arc<std::sync::atomic::AtomicUsize>,
    slots: Arc<std::sync::atomic::AtomicUsize>,
    fetches: SharedFetchRegistry,
}
#[derive(Default)]
struct NativeCssDependencies(
    RefCell<Vec<crate::executable::CssDependencyBatch>>,
    std::cell::Cell<Option<std::time::Instant>>,
);

// Standalone CSP reports already have exact native Document ownership. Mirror
// successful DOM-event admission into a bounded native trace, without touching
// author properties or consuming the event/reporting queue a second time.
const NATIVE_CSP_TRACE_COUNT: usize = 256;
const NATIVE_CSP_TRACE_BYTES: usize = 8 * 1024 * 1024;
#[derive(Default)]
struct NativeCspTrace {
    entries: Vec<(String, String, String)>,
    bytes: usize,
    omitted: u64,
}
impl NativeCspTrace {
    fn record(&mut self, violation: &crate::csp::CspViolation) {
        let disposition = match violation.disposition {
            crate::csp::CspDisposition::Enforce => "enforce",
            crate::csp::CspDisposition::Report => "report",
        };
        let bytes = disposition
            .len()
            .saturating_add(violation.effective_directive.len())
            .saturating_add(violation.blocked_uri.len());
        if self.entries.len() >= NATIVE_CSP_TRACE_COUNT
            || self.bytes.saturating_add(bytes) > NATIVE_CSP_TRACE_BYTES
        {
            self.omitted = self.omitted.saturating_add(1);
            return;
        }
        self.entries.push((
            disposition.into(),
            violation.effective_directive.clone(),
            violation.blocked_uri.clone(),
        ));
        self.bytes += bytes;
    }
    fn take(&mut self) -> Vec<crate::executable::ContextTraceEntry> {
        let mut entries = std::mem::take(&mut self.entries).into_iter().map(|(disposition, effective_directive, blocked_uri)| {
            crate::executable::ContextTraceEntry {
                kind: "native_csp_violation".into(),
                detail: serde_json::json!({"disposition":disposition,"effective_directive":effective_directive,"blocked_uri":blocked_uri}),
            }
        }).collect::<Vec<_>>();
        self.bytes = 0;
        let omitted = std::mem::take(&mut self.omitted);
        if omitted != 0 {
            entries.push(crate::executable::ContextTraceEntry {
                kind: "native_csp_trace_overflow".into(),
                detail: serde_json::json!({"omitted":omitted}),
            });
        }
        entries
    }
}
struct NativeDocumentCspTrace {
    owner: std::rc::Weak<V8BindingState>,
    document: oryn_dom::NodeId,
    epoch: u64,
    trace: RefCell<NativeCspTrace>,
}
fn csp_trace_root<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    context: v8::Local<'s, v8::Context>,
) -> v8::Local<'s, v8::Context> {
    let window = context.global(scope);
    let top = oryn_js::v8_bindings::native_window_top(scope, window);
    top.get_creation_context(scope).unwrap_or(context)
}
// The report drain owns a contextless handle scope; other native callers may
// already have a context entered. Always enter the supplied report owner here.
fn record_native_csp_trace<'s, C>(
    scope: &mut v8::PinScope<'s, '_, C>,
    context: v8::Local<'s, v8::Context>,
    violation: &crate::csp::CspViolation,
) {
    let scope = &mut v8::ContextScope::new(scope, context);
    let Some(source) = context.get_slot::<V8BindingState>() else {
        return;
    };
    if source.binding_global() != oryn_js::v8_bindings::BindingGlobal::Window {
        return;
    }
    let root = csp_trace_root(scope, context);
    let Some(state) = root.get_slot::<V8BindingState>() else {
        return;
    };
    if state.execution_retired() {
        return;
    }
    let document = state.document_root();
    let epoch = state.document_authority_epoch();
    let trace = root
        .get_slot::<NativeDocumentCspTrace>()
        .filter(|trace| {
            trace.document == document
                && trace.epoch == epoch
                && trace
                    .owner
                    .upgrade()
                    .is_some_and(|owner| Rc::ptr_eq(&owner, &state))
        })
        .unwrap_or_else(|| {
            let trace = Rc::new(NativeDocumentCspTrace {
                owner: Rc::downgrade(&state),
                document,
                epoch,
                trace: RefCell::new(NativeCspTrace::default()),
            });
            root.set_slot(trace.clone());
            trace
        });
    trace.trace.borrow_mut().record(violation);
}

struct NativeScriptCspReports {
    document: oryn_dom::NodeId,
    epoch: u64,
    _owner: Option<crate::standalone_csp_reporting::Owner>,
    queue: Arc<crate::executable::module_graph_load::ScriptCspReports>,
}
fn native_script_report_queue(
    scope: &mut v8::PinScope,
) -> Arc<crate::executable::module_graph_load::ScriptCspReports> {
    let context = scope.get_current_context();
    let Some(state) = context.get_slot::<V8BindingState>() else {
        return Default::default();
    };
    let document = state.document_root();
    let epoch = state.document_authority_epoch();
    if let Some(reports) = context.get_slot::<NativeScriptCspReports>()
        && reports.document == document
        && reports.epoch == epoch
    {
        return reports.queue.clone();
    }
    let (owner, sender) = match current_fetch_endpoint(scope) {
        Some(fetch) => {
            let (owner, sender) = crate::standalone_csp_reporting::Sender::new(
                fetch.broker,
                fetch.base_url.to_string(),
            );
            (Some(owner), Some(sender))
        }
        None => (None, None),
    };
    let retirement_sender = sender.as_ref().map(Arc::downgrade);
    let queue = Arc::new(
        crate::executable::module_graph_load::ScriptCspReports::with_transport(
            sender,
            state.binding_global() == oryn_js::v8_bindings::BindingGlobal::Window,
        ),
    );
    let preloads = Arc::downgrade(&queue.3);
    state.set_script_report_retirement(Rc::new(move || {
        if let Some(sender) = retirement_sender
            .as_ref()
            .and_then(std::sync::Weak::upgrade)
        {
            sender.cancel();
        }
        if let Some(preloads) = preloads.upgrade() {
            preloads.retire();
        }
    }));
    context.set_slot(Rc::new(NativeScriptCspReports {
        document,
        epoch,
        _owner: owner,
        queue: queue.clone(),
    }));
    queue
}
struct ImportAdmission(Rc<std::cell::Cell<usize>>);
impl Drop for ImportAdmission {
    fn drop(&mut self) {
        self.0.set(self.0.get() - 1);
    }
}
struct PendingDynamicImport {
    phase: v8::ModuleImportPhase,
    graph_complete: bool,
    completion_deadline: Option<Instant>,
    native_failure: Option<crate::script_load_failure::ScriptLoadFailure>,
    graph_failure: Option<String>,
    loaded: Option<PreparedModuleImport>,
    settled: bool,
    retry: Option<Instant>,
    environment: EnvironmentDescriptor,
    epoch: u64,
    resolver: v8::Global<v8::PromiseResolver>,
    options: v8::Global<v8::Data>,
    resource: v8::Global<v8::Value>,
    specifier: v8::Global<v8::String>,
    attributes: v8::Global<v8::FixedArray>,
    graph: ModuleGraphLoad,
    backend: ModuleGraphBackend,
    deadline: Instant,
    _admission: ImportAdmission,
}
struct PendingDirectModule {
    environment: EnvironmentDescriptor,
    epoch: u64,
    retry: Option<Instant>,
    prepared: bool,
    native_failure: Option<crate::script_load_failure::ScriptLoadFailure>,
    name: String,
    source: oryn_js::script_source::ScriptSource,
    dependencies: BTreeMap<String, String>,
    report_errors: bool,
    cache_root: bool,
    request: String,
    graph: ModuleGraphLoad,
    backend: ModuleGraphBackend,
    deadline: Instant,
    _source: crate::executable::ScriptSourceReservation,
    _admission: ImportAdmission,
}
struct PreparedImportLink {
    root: RefCell<Option<String>>,
    error: Option<u64>,
}
fn prepared_import_root(scope: &mut v8::PinScope) -> Option<String> {
    scope
        .get_current_context()
        .get_slot::<PreparedImportLink>()
        .and_then(|entry| entry.root.borrow_mut().take())
}
fn dynamic_import<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    options: v8::Local<'s, v8::Data>,
    resource: v8::Local<'s, v8::Value>,
    specifier: v8::Local<'s, v8::String>,
    phase: v8::ModuleImportPhase,
    attributes: v8::Local<'s, v8::FixedArray>,
) -> Option<v8::Local<'s, v8::Promise>> {
    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    if phase == v8::ModuleImportPhase::kSource {
        let message = v8::String::new(scope, "this module type has no source representation")?;
        let error = v8::Exception::syntax_error(scope, message);
        resolver.reject(scope, error)?;
        return Some(promise);
    }
    let result = (|| -> Result<PendingDynamicImport, String> {
        let environment = current_environment(scope)?;
        let binding = scope.get_current_context().get_slot::<V8BindingState>();
        let epoch = binding
            .as_ref()
            .map_or(0, |binding| binding.document_authority_epoch());
        let kind = import_module_type(scope, attributes, false)?;
        let settings = v8::Local::<v8::FixedArray>::try_from(options)
            .ok()
            .filter(|options| options.length() == 4);
        let mut include = settings
            .and_then(|options| options.get(scope, 0))
            .and_then(|value| v8::Local::<v8::Value>::try_from(value).ok())
            .is_some_and(|value| value.is_boolean() && value.boolean_value(scope));
        let resource_text = resource.to_rust_string_lossy(scope);
        let policy = settings
            .and_then(|options| options.get(scope, 1))
            .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
            .map(|value| value.to_rust_string_lossy(scope))
            .unwrap_or_else(|| module_referrer_policy(scope, &resource_text));
        let base = settings
            .and_then(|options| options.get(scope, 2))
            .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
            .map(|value| value.to_rust_string_lossy(scope))
            .unwrap_or_else(|| resource.to_rust_string_lossy(scope));
        let muted = settings
            .and_then(|options| options.get(scope, 3))
            .and_then(|value| v8::Local::<v8::Value>::try_from(value).ok())
            .is_some_and(|value| value.is_boolean() && value.boolean_value(scope));
        if muted {
            return Err("dynamic import is unavailable from a muted script".into());
        }
        let requested = specifier.to_rust_string_lossy(scope);
        let agent = scope.get_slot::<AgentExecution>().cloned();
        if let Some(agent) = &agent {
            include = agent.bootstrap.options.credentials
                == oryn_js::worker_wire::WorkerCredentials::Include;
        }
        if agent
            .as_ref()
            .is_some_and(|agent| agent.bootstrap.kind == AgentKind::Service)
        {
            return Err("dynamic import is unavailable in service workers".into());
        }
        if agent.is_some() && kind == crate::ModuleType::Css {
            return Err("CSS modules are unavailable in worker globals".into());
        }
        let base = if Url::parse(&base).is_ok() {
            base
        } else if let Some(agent) = &agent {
            agent.bootstrap.script_url.clone()
        } else if let Some(binding) = &binding {
            oryn_js::v8_bindings::document_base_url(scope, binding, true)
                .map_err(|error| error.to_string())?
        } else if let Some(fetch) = current_fetch_endpoint(scope) {
            fetch.base_url.to_string()
        } else {
            base
        };
        let sources = scope
            .get_slot::<ModuleSources>()
            .map(|sources| sources.0.clone())
            .unwrap_or_default();
        #[cfg(unix)]
        let context_endpoint = current_context_endpoint(scope);
        #[cfg(unix)]
        let native_context = context_endpoint.is_some();
        #[cfg(not(unix))]
        let native_context = false;
        let fetch_endpoint = current_fetch_endpoint(scope);
        let native = agent.is_some() || native_context || fetch_endpoint.is_some();
        let map = module_import_map(scope);
        let resolved = if agent.is_some() {
            agent_module_url(&base, &requested).ok_or("invalid worker module specifier")?
        } else {
            let resolved = Url::parse(&base)
                .ok()
                .map(|base| map.borrow().resolve(&base, &requested))
                .transpose()?
                .flatten();
            resolved
                .or_else(|| {
                    (!native)
                        .then(|| resolve_module_name(&base, &requested, &sources))
                        .flatten()
                })
                .ok_or("module specifier cannot be resolved")?
        };
        if resolved
            .len()
            .saturating_add(base.len())
            .saturating_add(policy.len())
            > 192 * 1024
        {
            return Err("dynamic import metadata limit exceeded".into());
        }
        if let Ok(base_url) = Url::parse(&base) {
            map.borrow_mut().record_resolution(&base_url, &requested)?;
        }
        let integrity = map.borrow().integrity_for(&resolved).map(str::to_owned);
        let captured_options =
            oryn_js::v8_bindings::script_import_options(scope, include, &policy, &base, false)
                .map_err(|error| error.to_string())?;
        let captured_resource = v8::String::new(scope, &base).ok_or("import base allocation")?;
        let captured_specifier =
            v8::String::new(scope, &resolved).ok_or("import URL allocation")?;
        let registry = scope
            .get_slot::<Rc<RefCell<DynamicImports>>>()
            .ok_or("import scheduler absent")?
            .clone();
        let state = registry.borrow_mut();
        if state.admitted.get() >= 256 {
            return Err("dynamic import capacity exceeded".into());
        }
        #[cfg(unix)]
        let window_backend = context_endpoint
            .map(|endpoint| -> Result<ModuleGraphBackend, String> {
                let binding = binding
                    .as_ref()
                    .ok_or("native window import binding unavailable")?;
                let creator = binding
                    .native_worker_creator()
                    .map_err(|error| error.to_string())?;
                let initiator = binding
                    .request_initiator_url()
                    .map_err(|error| error.to_string())?;
                Ok(ModuleGraphBackend::Window(
                    endpoint.client,
                    endpoint.page_id,
                    Some((creator, initiator)),
                    binding
                        .capture_script_policy()
                        .map_err(|error| error.to_string())?,
                ))
            })
            .transpose()?;
        #[cfg(not(unix))]
        let window_backend: Option<ModuleGraphBackend> = None;
        let backend = if let Some(agent) = agent {
            ModuleGraphBackend::Worker {
                transport: agent.transport,
                bootstrap: agent.bootstrap,
            }
        } else if let Some(backend) = window_backend {
            backend
        } else if let Some(fetch) = fetch_endpoint {
            let reports = native_script_report_queue(scope);
            let initiator = binding
                .as_ref()
                .and_then(|binding| binding.request_initiator_url().ok())
                .unwrap_or_else(|| fetch.base_url.to_string());
            ModuleGraphBackend::Broker {
                coep: scope
                    .get_current_context()
                    .get_slot::<crate::standalone_document_policy::DocumentPolicy>()
                    .map(|policy| crate::coep_reporting::Policy::new(&initiator, &policy.headers))
                    .unwrap_or_default(),
                broker: fetch.broker,
                base: fetch.base_url,
                initiator,
                isolation: scope
                    .get_current_context()
                    .get_slot::<crate::agent_isolation::DocumentIsolationPolicy>()
                    .map(|policy| *policy)
                    .unwrap_or_default(),
                csp: binding
                    .as_ref()
                    .map(|binding| binding.capture_local_script_policy())
                    .transpose()
                    .map_err(|error| error.to_string())?
                    .flatten(),
                reports,
            }
        } else {
            let reports = native_script_report_queue(scope);
            ModuleGraphBackend::Sources(
                sources,
                binding
                    .as_ref()
                    .map(|binding| binding.capture_local_script_policy())
                    .transpose()
                    .map_err(|error| error.to_string())?
                    .flatten(),
                reports,
            )
        };
        let limits = scope
            .get_current_context()
            .get_slot::<DynamicImportLimits>();
        let (bytes, slots, fetches) = limits
            .map(|limits| {
                (
                    limits.bytes.clone(),
                    limits.slots.clone(),
                    limits.fetches.clone(),
                )
            })
            .unwrap_or_else(|| {
                (
                    state.bytes.clone(),
                    state.slots.clone(),
                    state.fetches.clone(),
                )
            });
        let graph = ModuleGraphLoad::external_root(crate::executable::module_graph_load::ExternalModuleRootInput { requested: &resolved, parent: &base, kind, include, policy: &policy, integrity, budget: bytes, slots, registry: fetches })
        .map_err(|error| error.to_string())?;
        let captured_options: v8::Local<v8::Data> = captured_options.into();
        let captured_resource: v8::Local<v8::Value> = captured_resource.into();
        state.admitted.set(state.admitted.get() + 1);
        Ok(PendingDynamicImport {
            phase,
            graph_complete: false,
            completion_deadline: None,
            native_failure: None,
            graph_failure: None,
            loaded: None,
            settled: false,
            retry: None,
            environment,
            epoch,
            resolver: v8::Global::new(scope, resolver),
            options: v8::Global::new(scope, captured_options),
            resource: v8::Global::new(scope, captured_resource),
            specifier: v8::Global::new(scope, captured_specifier),
            attributes: v8::Global::new(scope, attributes),
            graph,
            backend,
            deadline: Instant::now() + Duration::from_secs(60),
            _admission: ImportAdmission(state.admitted.clone()),
        })
    })();
    match result {
        Ok(pending) => scope
            .get_slot::<Rc<RefCell<DynamicImports>>>()?
            .borrow_mut()
            .pending
            .push_back(pending),
        Err(error) => {
            let message = v8::String::new(scope, &error)?;
            let error = v8::Exception::type_error(scope, message);
            resolver.reject(scope, error);
        }
    }
    Some(promise)
}
struct NativeModuleGraphRetry(std::cell::Cell<Option<Instant>>);
include!("native_owned_css_dependency_queue.rs");
impl RawV8Host {
    pub(crate) fn schedule_module_graph_retry(&mut self, retry: Instant) {
        let Some(context) = self.contexts.get(&self.active_realm).cloned() else {
            return;
        };
        v8::scope!(let scope,&mut self.isolate);
        let context = v8::Local::new(scope, &context);
        if let Some(wakeup) = context.get_slot::<NativeModuleGraphRetry>() {
            wakeup.0.set(Some(
                wakeup
                    .0
                    .get()
                    .filter(|old| *old > Instant::now())
                    .map_or(retry, |old| old.min(retry)),
            ));
        } else {
            context.set_slot(Rc::new(NativeModuleGraphRetry(std::cell::Cell::new(Some(
                retry,
            )))));
        }
    }
    fn module_graph_retry_delay(&mut self) -> Option<Duration> {
        let mut delay = None;
        let now = Instant::now();
        for realm in self.active_root_realm_ids() {
            if self.realm_is_parked(realm) {
                continue;
            }
            let Some(context) = self.contexts.get(&realm).cloned() else {
                continue;
            };
            v8::scope!(let scope,&mut self.isolate);
            let context = v8::Local::new(scope, &context);
            if let Some(wakeup) = context.get_slot::<NativeModuleGraphRetry>()
                && let Some(retry) = wakeup.0.get()
            {
                if retry <= now {
                    wakeup.0.set(None);
                }
                let next = retry.saturating_duration_since(now);
                delay = Some(delay.map_or(next, |old: Duration| old.min(next)));
            }
        }
        delay
    }

    pub(crate) fn native_css_resource_layout(
        &self,
        realm: RealmId,
    ) -> Result<oryn_js::v8_bindings::BindingLayout, HostError> {
        self.binding_states
            .get(&realm)
            .ok_or_else(|| HostError::Execution("CSS demand realm retired".into()))?
            .native_resource_layout()
            .map_err(HostError::Execution)
    }
    pub(crate) fn css_font_owner_applicable(
        &self,
        realm: RealmId,
        node: oryn_dom::NodeId,
    ) -> Result<bool, HostError> {
        let state = self
            .binding_states
            .get(&realm)
            .ok_or_else(|| HostError::Execution("font realm retired".into()))?;
        let dom = self
            .automation_dom(realm)
            .ok_or_else(|| HostError::Execution("font document retired".into()))?;
        let media = {
            let dom = dom.borrow();
            if dom
                .arena()
                .stylesheet_disabled(node)
                .map_err(|error| HostError::Execution(error.to_string()))?
            {
                return Ok(false);
            }
            dom.arena()
                .stylesheet_media(node)
                .map_err(|error| HostError::Execution(error.to_string()))?
        };
        state
            .native_media_matches(&media)
            .map_err(HostError::Execution)
    }
    pub(crate) fn install_native_css_font(
        &mut self,
        realm: RealmId,
        document: oryn_dom::NodeId,
        owner: oryn_dom::NodeId,
        revision: u64,
        font: oryn_js::v8_bindings::BindingFontResource,
        custody: std::sync::Arc<dyn std::any::Any + Send + Sync>,
    ) -> Result<(), HostError> {
        let state = self
            .binding_states
            .get(&realm)
            .ok_or_else(|| HostError::Execution("font realm retired".into()))?;
        state
            .install_native_font(document, owner, revision, font, custody)
            .map_err(HostError::Execution)
    }

    pub(crate) fn queue_css_dependencies(
        &mut self,
        node: oryn_dom::NodeId,
        revision: u64,
        dependencies: crate::executable::CssDependencies,
    ) -> Result<(), HostError> {
        let epoch = self
            .document_stream_epoch(self.active_realm)
            .ok_or_else(|| HostError::Execution("CSS dependency owner retired".into()))?;
        let context = self
            .contexts
            .get(&self.active_realm)
            .cloned()
            .ok_or_else(|| HostError::Execution("CSS dependency context absent".into()))?;
        v8::scope!(let scope,&mut self.isolate);
        let context = v8::Local::new(scope, &context);
        let limits = context
            .get_slot::<DynamicImportLimits>()
            .ok_or_else(|| HostError::Execution("CSS dependency budget absent".into()))?;
        if context.get_slot::<NativeCssDependencies>().is_none() {
            context.set_slot(Rc::new(NativeCssDependencies::default()));
        }
        let queue = context
            .get_slot::<NativeCssDependencies>()
            .expect("native CSS queue");
        let mut queue = queue.0.borrow_mut();
        if queue.len() >= 1024 {
            return Err(HostError::LimitExceeded {
                resource: "native CSS dependency batches".into(),
            });
        }
        let size = dependencies
            .iter()
            .fold(0usize, |bytes, (base, initiator, policy, css)| {
                bytes
                    .saturating_add(base.len())
                    .saturating_add(initiator.len())
                    .saturating_add(policy.len())
                    .saturating_add(css.len())
            });
        let reservation = crate::executable::ScriptSourceReservation::reserve(&limits.bytes, size)
            .ok_or_else(|| HostError::LimitExceeded {
                resource: "queued CSS dependency bytes".into(),
            })?;
        queue.push(crate::executable::CssDependencyBatch {
            node,
            revision,
            epoch,
            dependencies,
            _bytes: reservation,
        });
        Ok(())
    }
    pub(crate) fn take_css_dependencies(
        &mut self,
    ) -> Vec<(RealmId, crate::executable::CssDependencyBatch)> {
        let roots = self.active_root_realm_ids();
        let mut pending = Vec::new();
        for realm in roots {
            if self.realm_is_parked(realm) {
                continue;
            }
            let epoch = self.document_stream_epoch(realm);
            let context = self.contexts.get(&realm).cloned();
            let Some(context) = context else { continue };
            v8::scope!(let scope,&mut self.isolate);
            let context = v8::Local::new(scope, &context);
            if let Some(queue) = context.get_slot::<NativeCssDependencies>() {
                if queue
                    .1
                    .get()
                    .is_some_and(|retry| retry > std::time::Instant::now())
                {
                    continue;
                }
                queue.1.set(None);
                pending.extend(
                    queue
                        .0
                        .borrow_mut()
                        .drain(..)
                        .filter(|batch| Some(batch.epoch) == epoch)
                        .map(|batch| (realm, batch)),
                );
            }
        }
        pending
    }
    pub(crate) fn restore_css_dependency(
        &mut self,
        realm: RealmId,
        batch: crate::executable::CssDependencyBatch,
    ) {
        // Admission between take/restore is native-only. The original queue
        // retains the capacity freed by drain; no author can replace this realm.
        if self.document_stream_epoch(realm) != Some(batch.epoch) {
            return;
        }
        let Some(context) = self.contexts.get(&realm).cloned() else {
            return;
        };
        v8::scope!(let scope,&mut self.isolate);
        let context = v8::Local::new(scope, &context);
        let Some(queue) = context.get_slot::<NativeCssDependencies>() else {
            return;
        };
        queue.0.borrow_mut().push(batch);
        queue.1.set(Some(
            std::time::Instant::now() + std::time::Duration::from_millis(25),
        ));
    }
    fn css_dependency_delay(&mut self) -> Option<std::time::Duration> {
        let now = std::time::Instant::now();
        let mut delay = None;
        for realm in self.active_root_realm_ids() {
            if self.realm_is_parked(realm) {
                continue;
            }
            let Some(context) = self.contexts.get(&realm).cloned() else {
                continue;
            };
            v8::scope!(let scope,&mut self.isolate);
            let context = v8::Local::new(scope, &context);
            if let Some(queue) = context.get_slot::<NativeCssDependencies>()
                && !queue.0.borrow().is_empty()
                && let Some(retry) = queue.1.get()
            {
                let next = retry.saturating_duration_since(now);
                delay = Some(delay.map_or(next, |old: std::time::Duration| old.min(next)));
            }
        }
        delay
    }
    fn has_css_dependencies(&mut self) -> bool {
        for realm in self.active_root_realm_ids() {
            if self.realm_is_parked(realm) {
                continue;
            }
            let epoch = self.document_stream_epoch(realm);
            let Some(context) = self.contexts.get(&realm).cloned() else {
                continue;
            };
            v8::scope!(let scope,&mut self.isolate);
            let context = v8::Local::new(scope, &context);
            if context
                .get_slot::<NativeCssDependencies>()
                .is_some_and(|queue| {
                    queue
                        .1
                        .get()
                        .is_none_or(|retry| retry <= std::time::Instant::now())
                        && queue
                            .0
                            .borrow()
                            .iter()
                            .any(|batch| Some(batch.epoch) == epoch)
                })
            {
                return true;
            }
        }
        false
    }
    pub(crate) fn authorize_captured_local_script(
        &mut self,
        policy: &crate::csp::CspPolicySet,
        request: &crate::csp::CspRequest,
    ) -> Result<bool, HostError> {
        self.authorize_captured_local_script_kind(policy, request, false)
    }
    pub(crate) fn authorize_captured_local_script_kind(
        &mut self,
        policy: &crate::csp::CspPolicySet,
        request: &crate::csp::CspRequest,
        speculation_rules: bool,
    ) -> Result<bool, HostError> {
        let reports = self.script_report_queue()?;
        let decision = if speculation_rules {
            policy.authorize_speculation_rules(request)
        } else {
            policy.authorize(request)
        };
        reports
            .push_with_policy(policy, decision.violations)
            .map_err(HostError::Execution)?;
        Ok(decision.allowed
            && !policy
                .sandbox()
                .is_some_and(|sandbox| !sandbox.allow_scripts))
    }
    pub(crate) fn has_module_resource_broker(&self) -> bool {
        self.endpoints_for_realm(self.active_realm)
            .is_some_and(|endpoints| endpoints.fetch.is_some())
    }
    pub(crate) fn take_native_csp_trace_entries(
        &mut self,
    ) -> Vec<crate::executable::ContextTraceEntry> {
        let Some(context) = self.contexts.get(&self.active_realm).cloned() else {
            return Vec::new();
        };
        v8::scope!(let scope, &mut self.isolate);
        let context = v8::Local::new(scope, &context);
        let scope = &mut v8::ContextScope::new(scope, context);
        let Some(source) = context.get_slot::<V8BindingState>() else {
            return Vec::new();
        };
        if source.binding_global() != oryn_js::v8_bindings::BindingGlobal::Window {
            return Vec::new();
        }
        let root = csp_trace_root(scope, context);
        let Some(state) = root.get_slot::<V8BindingState>() else {
            return Vec::new();
        };
        let Some(trace) = root.get_slot::<NativeDocumentCspTrace>() else {
            return Vec::new();
        };
        let entries = trace.trace.borrow_mut().take();
        if state.execution_retired()
            || trace.document != state.document_root()
            || trace.epoch != state.document_authority_epoch()
            || !trace
                .owner
                .upgrade()
                .is_some_and(|owner| Rc::ptr_eq(&owner, &state))
        {
            return Vec::new();
        }
        entries
    }
    pub(crate) fn script_report_queue(
        &mut self,
    ) -> Result<Arc<crate::executable::module_graph_load::ScriptCspReports>, HostError> {
        let context = self
            .contexts
            .get(&self.active_realm)
            .cloned()
            .ok_or_else(|| HostError::Execution("script report realm absent".into()))?;
        v8::scope!(let scope,&mut self.isolate);
        let context = v8::Local::new(scope, &context);
        let scope = &mut v8::ContextScope::new(scope, context);
        Ok(native_script_report_queue(scope))
    }
    fn publish_native_resource_timings(&mut self) {
        #[cfg(unix)]
        self.publish_external_resource_timings();
        for (realm, context) in self.contexts.clone() {
            if self.realm_is_parked(realm) {
                continue;
            }
            let Some(state) = self.binding_states.get(&realm) else {
                continue;
            };
            v8::scope!(let scope,&mut self.isolate);
            let context = v8::Local::new(scope, &context);
            let Some(reports) = context.get_slot::<NativeScriptCspReports>() else {
                continue;
            };
            let timings = reports.queue.take_timings();
            if state.execution_retired()
                || reports.document != state.document_root()
                || reports.epoch != state.document_authority_epoch()
            {
                continue;
            }
            for timing in timings {
                state.record_external_resource_timing_with_status(
                    timing.name,
                    timing.initiator,
                    timing.started,
                    timing.finished,
                    timing.details,
                    timing.render_blocking,
                );
            }
        }
    }
    fn has_background_script_reports(&mut self) -> bool {
        for (realm, context) in self.contexts.clone() {
            if self.realm_is_parked(realm) {
                continue;
            }
            let Some(state) = self.binding_states.get(&realm) else {
                continue;
            };
            v8::scope!(let scope,&mut self.isolate);
            let context = v8::Local::new(scope, &context);
            if context
                .get_slot::<NativeScriptCspReports>()
                .is_some_and(|reports| {
                    !state.execution_retired()
                        && reports.document == state.document_root()
                        && reports.epoch == state.document_authority_epoch()
                        && reports.queue.pending()
                })
            {
                return true;
            }
        }
        false
    }
    fn drain_background_script_reports(&mut self) -> Result<(), HostError> {
        for (realm, context) in self.contexts.clone() {
            if self.realm_is_parked(realm) {
                continue;
            }
            let Some(state) = self.binding_states.get(&realm) else {
                continue;
            };
            v8::scope!(let scope,&mut self.isolate);
            let context = v8::Local::new(scope, &context);
            let Some(reports) = context.get_slot::<NativeScriptCspReports>() else {
                continue;
            };
            if state.execution_retired()
                || reports.document != state.document_root()
                || reports.epoch != state.document_authority_epoch()
            {
                let _ = reports.queue.take();
                let _ = reports.queue.take_timings();
                continue;
            }
            for timing in reports.queue.take_timings() {
                state.record_external_resource_timing_with_status(
                    timing.name,
                    timing.initiator,
                    timing.started,
                    timing.finished,
                    timing.details,
                    timing.render_blocking,
                );
            }
            let pending = reports.queue.take().map_err(HostError::Execution)?;
            let mut pending = pending.into_iter();
            while let Some(violation) = pending.next() {
                if let Err(error) = queue_standalone_csp_violation(state, violation.clone()) {
                    let mut retained = vec![violation];
                    retained.extend(pending);
                    reports.queue.push(retained).map_err(HostError::Execution)?;
                    return Err(HostError::Execution(error));
                }
                record_native_csp_trace(scope, context, &violation);
            }
        }
        Ok(())
    }
    pub(crate) fn standalone_document_policy(
        &mut self,
    ) -> Result<crate::standalone_document_policy::DocumentPolicy, HostError> {
        let context = self
            .contexts
            .get(&self.active_realm)
            .cloned()
            .ok_or_else(|| HostError::Execution("document policy realm absent".into()))?;
        v8::scope!(let scope,&mut self.isolate);
        let context = v8::Local::new(scope, &context);
        if let Some(policy) =
            context.get_slot::<crate::standalone_document_policy::DocumentPolicy>()
        {
            return Ok((*policy).clone());
        }
        let origin = context
            .get_slot::<RealmDescriptor>()
            .ok_or_else(|| HostError::Execution("document policy origin absent".into()))?
            .origin
            .serialize();
        let mut headers = Vec::new();
        if let Some(policy) = context.get_slot::<crate::agent_isolation::DocumentIsolationPolicy>()
        {
            policy.append_native_headers(&mut headers);
        }
        Ok(crate::standalone_document_policy::DocumentPolicy::root(
            origin, &headers,
        ))
    }
    pub(crate) fn set_standalone_document_policy(
        &mut self,
        mut policy: crate::standalone_document_policy::DocumentPolicy,
    ) -> Result<(), HostError> {
        let isolation = policy.isolation();
        {
            let context = self
                .contexts
                .get(&self.active_realm)
                .cloned()
                .ok_or_else(|| HostError::Execution("document policy realm absent".into()))?;
            v8::scope!(let scope,&mut self.isolate);
            let context = v8::Local::new(scope, &context);
            let scope = &mut v8::ContextScope::new(scope, context);
            let origin = context
                .get_slot::<RealmDescriptor>()
                .ok_or_else(|| HostError::Execution("document policy origin absent".into()))?
                .origin
                .clone();
            policy.rebase_origin(origin.serialize());
            let policy = Rc::new(policy);
            #[cfg(unix)]
            let local_policy = endpoints_for_context(scope, context)
                .and_then(|endpoints| endpoints.context)
                .is_none();
            #[cfg(not(unix))]
            let local_policy = true;
            if let Some(state) = context
                .get_slot::<V8BindingState>()
                .filter(|_| local_policy)
            {
                admit_local_origin_cluster(scope, context, &state, &policy.headers)?;
                let has_parent = context.get_slot::<FrameLifetime>().is_some();
                let parent = context.get_slot::<FrameLifetime>().and_then(|frame| {
                    let parent = frame.parent.upgrade()?;
                    let parent_context = scope
                        .get_slot::<NativeRealmIdentities>()?
                        .contexts
                        .get(&frame.parent_environment.realm)?
                        .clone();
                    let parent_context = parent_context.to_local(scope)?;
                    let parent_policy = parent_context
                        .get_slot::<crate::standalone_document_policy::DocumentPolicy>(
                    )?;
                    Some((Rc::downgrade(&parent), frame.frame, parent_policy))
                });
                let snapshot_policy = policy.clone();
                let snapshot_parent = parent.clone();
                state.set_native_permissions_policy_snapshot(Rc::new(move || {
                    let Some((parent, node, _)) = &snapshot_parent else {
                        if has_parent {
                            return Err("InvalidStateError: policy parent retired".into());
                        }
                        return Ok(snapshot_policy.snapshot());
                    };
                    let parent = parent
                        .upgrade()
                        .filter(|parent| !parent.execution_retired())
                        .ok_or("InvalidStateError: policy parent retired")?;
                    let (allow, source) = {
                        let dom = parent.dom();
                        let dom = dom.borrow();
                        let allow = dom
                            .frame_permissions_policy_allow(*node)
                            .map_err(|error| error.to_string())?;
                        let source = if dom
                            .attribute_in_null_namespace(*node, "srcdoc")
                            .map_err(|error| error.to_string())?
                            .is_some()
                        {
                            "about:srcdoc"
                        } else {
                            dom.attribute_in_null_namespace(*node, "src")
                                .map_err(|error| error.to_string())?
                                .unwrap_or("about:blank")
                        };
                        (allow, source.to_owned())
                    };
                    let source = parent
                        .native_document_base_url(true)
                        .ok()
                        .and_then(|base| Url::parse(&base).ok())
                        .and_then(|base| base.join(&source).ok())
                        .map(|url| url.to_string())
                        .unwrap_or(source);
                    let parent = parent
                        .native_permissions_policy_snapshot()
                        .map_err(|error| error.to_string())?;
                    Ok(parent.child(
                        snapshot_policy.origin.clone(),
                        snapshot_policy.headers.clone(),
                        allow,
                        source,
                    ))
                }));
                let permissions = policy.clone();
                let parent_permissions = parent.clone();
                state.set_native_permissions_policy(Rc::new(move |feature| {
                    if !permissions.own_allows(feature) {
                        return false;
                    }
                    let Some((parent, node, parent_policy)) = &parent_permissions else {
                        return !has_parent;
                    };
                    let Some(parent) = parent
                        .upgrade()
                        .filter(|parent| !parent.execution_retired())
                    else {
                        return false;
                    };
                    if !parent.native_permissions_policy_allows(feature) {
                        return false;
                    }
                    let attributes = {
                        let dom = parent.dom();
                        let dom = dom.borrow();
                        let Ok(allow) = dom.frame_permissions_policy_allow(*node) else {
                            return false;
                        };
                        let source = if dom
                            .attribute_in_null_namespace(*node, "srcdoc")
                            .ok()
                            .flatten()
                            .is_some()
                        {
                            "about:srcdoc"
                        } else {
                            dom.attribute_in_null_namespace(*node, "src")
                                .ok()
                                .flatten()
                                .unwrap_or("about:blank")
                        };
                        (allow, source.to_owned())
                    };
                    let source = parent
                        .native_document_base_url(true)
                        .ok()
                        .and_then(|base| Url::parse(&base).ok())
                        .and_then(|base| base.join(&attributes.1).ok())
                        .map(|url| url.to_string())
                        .unwrap_or(attributes.1);
                    parent_policy.edge_allows(feature, &permissions.origin, &attributes.0, &source)
                }));
                let reports = policy.clone();
                state.set_native_permissions_report_only(Rc::new(move |feature| {
                    reports.own_report_only_allows(feature)
                        && (!has_parent
                            || parent.as_ref().is_some_and(|(parent, _, policy)| {
                                parent.upgrade().is_some_and(|parent| {
                                    !parent.execution_retired()
                                        && parent.native_permissions_report_only_allows(feature)
                                        && policy.edge_report_only_allows(feature, &reports.origin)
                                })
                            }))
                }));
            }
            context.set_slot(policy);
        }
        self.set_script_isolation_policy(isolation)
    }
    pub(crate) fn standalone_resource_policy(
        &mut self,
    ) -> Result<Option<crate::csp::CspPolicySet>, HostError> {
        let context = self
            .contexts
            .get(&self.active_realm)
            .cloned()
            .ok_or_else(|| HostError::Execution("resource policy realm absent".into()))?;
        v8::scope!(let scope,&mut self.isolate);
        let context = v8::Local::new(scope, &context);
        Ok(context
            .get_slot::<RefCell<crate::csp::CspPolicySet>>()
            .map(|policy| policy.borrow().clone()))
    }
    pub(crate) fn set_standalone_resource_policy(
        &mut self,
        mut policy: crate::csp::CspPolicySet,
    ) -> Result<(), HostError> {
        self.constrain_automatic_features_allowed(
            policy.sandbox().is_none_or(|sandbox| sandbox.allow_scripts),
        )?;
        self.constrain_storage_access_allowed(
            policy
                .sandbox()
                .is_none_or(|sandbox| sandbox.allow_storage_access_by_user_activation),
        )?;
        let context = self
            .contexts
            .get(&self.active_realm)
            .cloned()
            .ok_or_else(|| HostError::Execution("resource policy realm absent".into()))?;
        v8::scope!(let scope,&mut self.isolate);
        let context = v8::Local::new(scope, &context);
        let scope = &mut v8::ContextScope::new(scope, context);
        let descriptor = context
            .get_slot::<RealmDescriptor>()
            .ok_or_else(|| HostError::Execution("resource policy native origin absent".into()))?;
        policy.set_document_self_origin(&descriptor.origin.serialize());
        let prepared = context
            .get_slot::<V8BindingState>()
            .map(|state| prepare_local_document_policy(scope, &state, &policy))
            .transpose()?
            .flatten();
        if let Some(state) = context.get_slot::<V8BindingState>() {
            state
                .dom()
                .borrow_mut()
                .set_document_nonce_hiding(state.document_root(), policy.has_header_policy())
                .map_err(|error| HostError::Execution(error.to_string()))?;
        }
        let policy = Rc::new(RefCell::new(policy));
        context.set_slot(policy.clone());
        if let Some(state) = context.get_slot::<V8BindingState>() {
            install_local_document_provenance(scope, context, &state)?;
            let metadata = policy.clone();
            state.set_meta_csp_policy(Rc::new(move |state, source| {
                let previous = metadata.borrow().clone();
                metadata.borrow_mut().install_meta(source);
                if let Err(error) = state.refresh_document_provenance() {
                    *metadata.borrow_mut() = previous;
                    return Err(error.to_string());
                }
                Ok(())
            }));
            let reports = native_script_report_queue(scope);
            let inline_reports = reports.clone();
            let capture = policy.clone();
            let inline = policy.clone();
            let cached = RefCell::new(None);
            state.set_local_script_policy(
                Rc::new(move || capture_standalone_script_policy(&capture, &cached)),
                Rc::new(
                    move |state, source, nonce, parser_inserted, speculation_rules| {
                        authorize_standalone_inline_script(
                            state,
                            &inline.borrow(),
                            source,
                            nonce,
                            parser_inserted,
                            speculation_rules,
                            &inline_reports,
                        )
                    },
                ),
            );
            state.set_inline_style_policy(Rc::new(move |state, source, nonce, attribute| {
                authorize_standalone_inline_style(
                    state,
                    &policy.borrow(),
                    source,
                    nonce,
                    attribute,
                    &reports,
                )
            }));
            if let Some(prepared) = prepared {
                state
                    .install_document_provenance(prepared)
                    .map_err(|error| HostError::Execution(error.to_string()))?;
            }
        }
        Ok(())
    }
    pub(crate) fn constrain_standalone_frame_sandbox(
        &mut self,
        sandbox: Option<&str>,
    ) -> Result<(), HostError> {
        let Some(sandbox) = sandbox else {
            return Ok(());
        };
        let mut policy = self.standalone_resource_policy()?.ok_or_else(|| {
            HostError::Execution("standalone frame resource policy absent".into())
        })?;
        policy.install(
            &format!("sandbox {sandbox}"),
            crate::csp::CspDisposition::Enforce,
            "frame-container",
        );
        self.set_standalone_resource_policy(policy)
    }
    pub(crate) fn inherit_standalone_resource_policy(
        &mut self,
        parent: RealmId,
        url: &str,
        inherit: bool,
        allow: &str,
    ) -> Result<(), HostError> {
        let (parent_policy, parent_document, child_origin) = {
            let parent = self
                .contexts
                .get(&parent)
                .cloned()
                .ok_or_else(|| HostError::Execution("resource policy parent absent".into()))?;
            let child = self
                .contexts
                .get(&self.active_realm)
                .cloned()
                .ok_or_else(|| HostError::Execution("resource policy child absent".into()))?;
            v8::scope!(let scope,&mut self.isolate);
            let parent = v8::Local::new(scope, &parent);
            let child = v8::Local::new(scope, &child);
            let parent_origin = parent
                .get_slot::<RealmDescriptor>()
                .ok_or_else(|| HostError::Execution("parent policy origin absent".into()))?
                .origin
                .serialize();
            let parent_document = parent
                .get_slot::<crate::standalone_document_policy::DocumentPolicy>()
                .map(|policy| (*policy).clone())
                .unwrap_or_else(|| {
                    let mut headers = Vec::new();
                    if let Some(policy) =
                        parent.get_slot::<crate::agent_isolation::DocumentIsolationPolicy>()
                    {
                        policy.append_native_headers(&mut headers);
                    }
                    crate::standalone_document_policy::DocumentPolicy::root(parent_origin, &headers)
                });
            let policy = parent
                .get_slot::<RefCell<crate::csp::CspPolicySet>>()
                .map(|policy| policy.borrow().clone())
                .unwrap_or_else(|| crate::csp::CspPolicySet::new(url));
            let child_origin = child
                .get_slot::<RealmDescriptor>()
                .ok_or_else(|| HostError::Execution("child policy origin absent".into()))?
                .origin
                .serialize();
            (policy, parent_document, child_origin)
        };
        let mut policy = if inherit {
            parent_policy
        } else {
            crate::csp::CspPolicySet::new(url)
        };
        if inherit {
            policy.rebase_inherited_document(url);
        } else {
            policy.rebase_document(url);
        }
        let headers = if inherit {
            parent_document.headers.clone()
        } else {
            Vec::new()
        };
        let document = parent_document
            .child(child_origin, &headers, allow.to_owned(), url.to_owned())
            .map_err(HostError::Execution)?;
        self.set_standalone_resource_policy(policy)?;
        self.set_standalone_document_policy(document)
    }
    pub(crate) fn install_standalone_meta_policy(
        &mut self,
        serialized: &str,
    ) -> Result<(), HostError> {
        let mut policy = self
            .standalone_resource_policy()?
            .unwrap_or_else(|| crate::csp::CspPolicySet::new("about:blank"));
        policy.install_meta(serialized);
        self.set_standalone_resource_policy(policy)
    }
    pub(crate) fn module_fetch_backend(&mut self) -> Result<ModuleGraphBackend, HostError> {
        let context = self
            .contexts
            .get(&self.active_realm)
            .cloned()
            .ok_or_else(|| HostError::Execution("module source realm absent".into()))?;
        v8::scope!(let scope,&mut self.isolate);
        let context = v8::Local::new(scope, &context);
        let scope = &mut v8::ContextScope::new(scope, context);
        module_fetch_backend_in_scope(scope)
    }

    /// Direct native module execution owns graph discovery between host turns.
    /// Explicit in-memory hosts retain their synchronous, IO-free entry point.
    fn schedule_direct_module(
        &mut self,
        name: &str,
        source: oryn_js::script_source::ScriptSourceRef<'_>,
        dependencies: &BTreeMap<String, String>,
        report_errors: bool,
        cache_root: bool,
        request: &str,
    ) -> Result<bool, HostError> {
        let backend = self.module_fetch_backend()?;
        if matches!(backend, ModuleGraphBackend::Sources(..)) {
            return Ok(false);
        }
        let context = self
            .contexts
            .get(&self.active_realm)
            .cloned()
            .ok_or_else(|| HostError::Execution("direct module realm absent".into()))?;
        let (environment, epoch, registry, limits) = {
            v8::scope!(let scope,&mut self.isolate);
            let context = v8::Local::new(scope, &context);
            let scope = &mut v8::ContextScope::new(scope, context);
            let cache_key = (
                current_environment(scope).map_err(HostError::Execution)?.id,
                request.to_owned(),
            );
            if cache_root
                && scope
                    .get_slot::<ModuleCache>()
                    .is_some_and(|cache| cache.0.contains_key(&cache_key))
            {
                return Ok(false);
            }
            (
                current_environment(scope).map_err(HostError::Execution)?,
                context
                    .get_slot::<V8BindingState>()
                    .map_or(0, |binding| binding.document_authority_epoch()),
                scope
                    .get_slot::<Rc<RefCell<DynamicImports>>>()
                    .expect("import scheduler")
                    .clone(),
                context.get_slot::<DynamicImportLimits>(),
            )
        };
        let pending = registry.borrow_mut();
        if pending.admitted.get() >= 256 {
            return Err(HostError::LimitExceeded {
                resource: "direct module admission".into(),
            });
        }
        let (bytes, slots, fetches) = limits
            .map(|limits| {
                (
                    limits.bytes.clone(),
                    limits.slots.clone(),
                    limits.fetches.clone(),
                )
            })
            .unwrap_or_else(|| {
                (
                    pending.bytes.clone(),
                    pending.slots.clone(),
                    pending.fetches.clone(),
                )
            });
        let size = dependencies.iter().fold(
            name.len()
                .checked_add(source.raw().len())
                .and_then(|value| value.checked_add(request.len())),
            |size, (name, source)| {
                size.and_then(|size| size.checked_add(name.len()))
                    .and_then(|size| size.checked_add(source.len()))
            },
        );
        let size = size.ok_or_else(|| HostError::LimitExceeded {
            resource: "direct module source bytes".into(),
        })?;
        let reservation = crate::executable::ScriptSourceReservation::reserve(&bytes, size)
            .ok_or_else(|| HostError::LimitExceeded {
                resource: "direct module source bytes".into(),
            })?;
        let mut graph = ModuleGraphLoad::with_root_request(
            name,
            source,
            cache_root.then_some(request),
            bytes,
            slots,
            fetches,
        )?;
        graph.retain_document_authority();
        // Ready graphs may link immediately; poll is bounded and never waits
        // for IO. Cached or import-free modules retain synchronous semantics.
        drop(pending);
        let dependencies = dependencies.clone();
        match graph.poll_with_backend(self, &backend, &mut BTreeMap::new()) {
            Ok(true) if graph.error_token.is_some() => {
                return self
                    .execute_module_inner_with_started(
                        name,
                        source,
                        StartedModuleExecutionInput {
                            dependencies: &dependencies,
                            report_errors,
                            cache_root,
                            request_identity: request,
                            prepared: true,
                            graph_error: graph.error_token,
                            started: &mut || {},
                        },
                    )
                    .map(|_| true);
            }
            Ok(true) | Err(HostError::ModuleSyntax(_)) => return Ok(false),
            Ok(false) => {}
            Err(HostError::ScriptLoad(
                crate::script_load_failure::ScriptLoadFailure::Infrastructure(_),
            )) => {
                graph.defer_retry(self);
            }
            Err(error) => return Err(error),
        }
        let mut pending = registry.borrow_mut();
        pending.admitted.set(pending.admitted.get() + 1);
        let admission = ImportAdmission(pending.admitted.clone());
        pending.direct.push_back(PendingDirectModule {
            environment,
            epoch,
            retry: None,
            prepared: false,
            native_failure: None,
            name: name.into(),
            source: source.owned(),
            dependencies,
            report_errors,
            cache_root,
            request: request.into(),
            graph,
            backend,
            deadline: Instant::now() + Duration::from_secs(60),
            _source: reservation,
            _admission: admission,
        });
        Ok(true)
    }
    fn direct_module_environment_live(
        &self,
        environment: &EnvironmentDescriptor,
        epoch: u64,
    ) -> bool {
        self.import_realm_live(environment.realm)
            && self
                .document_authority_epoch(environment.realm)
                .unwrap_or(0)
                == epoch
            && self
                .isolate
                .get_slot::<AgentEnvironments>()
                .is_some_and(|registry| registry.is_live(environment))
    }
    fn poll_direct_modules(&mut self) -> Result<usize, HostError> {
        let registry = self
            .isolate
            .get_slot::<Rc<RefCell<DynamicImports>>>()
            .expect("import scheduler")
            .clone();
        let count = registry.borrow().direct.len().min(32);
        let previous = self.active_realm;
        let mut delivered = 0;
        for _ in 0..count {
            if self.worker_tasks_stopped() {
                break;
            }
            let Some(mut pending) = registry.borrow_mut().direct.pop_front() else {
                break;
            };
            if !self.direct_module_environment_live(&pending.environment, pending.epoch) {
                continue;
            }
            if self.realm_is_parked(pending.environment.realm) {
                registry.borrow_mut().direct.push_back(pending);
                continue;
            }
            self.active_realm = pending.environment.realm;
            if pending.retry.is_some_and(|retry| Instant::now() < retry) {
                registry.borrow_mut().direct.push_back(pending);
                continue;
            }
            if Instant::now() >= pending.deadline {
                if self.contexts.contains_key(&previous) {
                    self.active_realm = previous;
                }
                return Err(pending
                    .native_failure
                    .take()
                    .map(HostError::ScriptLoad)
                    .unwrap_or_else(|| {
                        HostError::Execution(
                            "direct module graph/preparation deadline exceeded".into(),
                        )
                    }));
            }
            let progress = if pending.prepared {
                Ok(true)
            } else {
                pending
                    .graph
                    .poll_with_backend(self, &pending.backend, &mut BTreeMap::new())
            };
            let mut started = false;
            let result = match progress {
                Ok(false) => {
                    registry.borrow_mut().direct.push_back(pending);
                    continue;
                }
                Ok(true) => {
                    pending.prepared = true;
                    self.execute_module_inner_with_started(
                        &pending.name,
                        &pending.source,
                        StartedModuleExecutionInput {
                            dependencies: &pending.dependencies,
                            report_errors: pending.report_errors,
                            cache_root: pending.cache_root,
                            request_identity: &pending.request,
                            prepared: true,
                            graph_error: pending.graph.error_token,
                            started: &mut || started = true,
                        },
                    )
                }
                Err(error) => Err(error),
            };
            match result {
                Ok(()) => delivered += 1,
                Err(HostError::ScriptLoad(
                    failure @ crate::script_load_failure::ScriptLoadFailure::Infrastructure(_),
                )) if !started => {
                    if pending.native_failure.is_none() {
                        pending.native_failure = Some(failure);
                    }
                    pending.retry = Some(Instant::now() + Duration::from_millis(10));
                    registry.borrow_mut().direct.push_back(pending);
                    continue;
                }
                Err(error) => {
                    if self.contexts.contains_key(&previous) {
                        self.active_realm = previous;
                    }
                    return Err(error);
                }
            }
        }
        if self.contexts.contains_key(&previous) {
            self.active_realm = previous;
        }
        Ok(delivered)
    }

    pub(crate) fn set_dynamic_import_limits(
        &mut self,
        realm: RealmId,
        bytes: Arc<std::sync::atomic::AtomicUsize>,
        slots: Arc<std::sync::atomic::AtomicUsize>,
        fetches: SharedFetchRegistry,
    ) -> Result<(), HostError> {
        let context = self
            .contexts
            .get(&realm)
            .cloned()
            .ok_or_else(|| HostError::Execution("import budget realm absent".into()))?;
        v8::scope!(let scope,&mut self.isolate);
        let context = v8::Local::new(scope, &context);
        context.set_slot(Rc::new(DynamicImportLimits {
            bytes,
            slots,
            fetches,
        }));
        Ok(())
    }

    pub(crate) fn resolve_explicit_module_name(
        &self,
        parent: &str,
        requested: &str,
        sources: &BTreeMap<String, String>,
    ) -> Option<String> {
        resolve_module_name(parent, requested, sources)
    }
    pub(crate) fn import_realm_live(&self, realm: RealmId) -> bool {
        self.contexts.contains_key(&realm)
            && self
                .binding_states
                .get(&realm)
                .is_none_or(|state| !state.execution_retired())
    }
    pub(crate) fn reserve_module_request_id(&self) -> Result<u64, HostError> {
        self.reserve_resource_request_ids(11)
    }
    pub(crate) fn reserve_resource_request_ids(&self, count: u64) -> Result<u64, HostError> {
        let fetch = self
            .endpoints_for_realm(self.active_realm)
            .and_then(|endpoint| endpoint.fetch)
            .ok_or_else(|| HostError::Execution("module broker endpoint retired".into()))?;
        let id = fetch.next_script_request.get();
        fetch
            .next_script_request
            .set(
                id.checked_add(count)
                    .ok_or_else(|| HostError::LimitExceeded {
                        resource: "resource request identifiers".into(),
                    })?,
            );
        Ok(id)
    }
    fn dynamic_import_completion_delay(&self) -> Option<Duration> {
        let registry = self
            .isolate
            .get_slot::<Rc<RefCell<DynamicImports>>>()?
            .borrow();
        let now = Instant::now();
        registry
            .pending
            .iter()
            .filter(|pending| !self.realm_is_parked(pending.environment.realm))
            .map(|pending| {
                if pending.graph_complete {
                    pending
                        .retry
                        .map_or(Duration::ZERO, |retry| retry.saturating_duration_since(now))
                } else {
                    pending
                        .retry
                        .unwrap_or(pending.deadline)
                        .min(pending.deadline)
                        .saturating_duration_since(now)
                }
            })
            .chain(
                registry
                    .direct
                    .iter()
                    .filter(|pending| !self.realm_is_parked(pending.environment.realm))
                    .map(|pending| {
                        pending
                            .retry
                            .unwrap_or(pending.deadline)
                            .min(pending.deadline)
                            .saturating_duration_since(now)
                    }),
            )
            .min()
    }
    fn has_pending_dynamic_imports(&self, runnable: bool) -> bool {
        if self
            .isolate
            .get_slot::<Rc<RefCell<ModuleSettlements>>>()
            .is_some_and(|jobs| {
                jobs.borrow().jobs.values().any(|job| {
                    self.direct_module_environment_live(&job.environment, job.epoch)
                        && !self.realm_is_parked(job.environment.realm)
                        && (!job.attached || job.outcome.is_some())
                        && (!runnable || job.retry <= Instant::now())
                })
            })
        {
            return true;
        }
        self.isolate
            .get_slot::<Rc<RefCell<DynamicImports>>>()
            .is_some_and(|registry| {
                registry.borrow().pending.iter().any(|pending| {
                    !self.realm_is_parked(pending.environment.realm)
                        && (!runnable
                            || (pending.retry.is_none_or(|retry| retry <= Instant::now())
                                && (pending.graph_complete || pending.graph.is_runnable()))
                            || (!pending.graph_complete && Instant::now() >= pending.deadline))
                }) || registry.borrow().direct.iter().any(|pending| {
                    !self.realm_is_parked(pending.environment.realm)
                        && (!runnable
                            || (pending.retry.is_none_or(|retry| retry <= Instant::now())
                                && (pending.prepared || pending.graph.is_runnable()))
                            || Instant::now() >= pending.deadline)
                })
            })
    }
    fn poll_dynamic_imports(&mut self) -> Result<usize, HostError> {
        let registry = self
            .isolate
            .get_slot::<Rc<RefCell<DynamicImports>>>()
            .expect("native import scheduler")
            .clone();
        let count = registry.borrow().pending.len().min(32);
        let previous = self.active_realm;
        let mut delivered = self.poll_direct_modules()?;
        for _ in 0..count {
            if self.worker_tasks_stopped() {
                break;
            }
            let Some(mut pending) = registry.borrow_mut().pending.pop_front() else {
                break;
            };
            let realm = pending.environment.realm;
            if !self.direct_module_environment_live(&pending.environment, pending.epoch) {
                continue;
            }
            if self.realm_is_parked(realm) {
                registry.borrow_mut().pending.push_back(pending);
                continue;
            }
            self.active_realm = realm;
            if pending.retry.is_some_and(|retry| retry > Instant::now()) {
                registry.borrow_mut().pending.push_back(pending);
                continue;
            }
            if !pending.graph_complete {
                if Instant::now() >= pending.deadline {
                    pending.graph_complete = true;
                    pending.graph_failure = Some(
                        pending
                            .native_failure
                            .take()
                            .map(|failure| failure.to_string())
                            .unwrap_or_else(|| "dynamic import graph deadline exceeded".into()),
                    );
                }
                if !pending.graph_complete {
                    match pending.graph.poll_with_backend(
                        self,
                        &pending.backend,
                        &mut BTreeMap::new(),
                    ) {
                        Ok(false) => {
                            registry.borrow_mut().pending.push_back(pending);
                            continue;
                        }
                        Ok(true) => pending.graph_complete = true,
                        Err(HostError::ScriptLoad(
                            failure @ (crate::script_load_failure::ScriptLoadFailure::Fetch(_)
                            | crate::script_load_failure::ScriptLoadFailure::Integrity(_)),
                        )) => {
                            pending.graph_complete = true;
                            pending.graph_failure = Some(failure.to_string());
                        }
                        Err(
                            error @ (HostError::ScriptLoad(
                                crate::script_load_failure::ScriptLoadFailure::Cancelled,
                            )
                            | HostError::LimitExceeded { .. }
                            | HostError::ScriptLoad(
                                crate::script_load_failure::ScriptLoadFailure::Capacity(_),
                            )),
                        ) => {
                            // Retirement is filtered before polling. A terminal failure
                            // in a still-live realm must settle its author promise.
                            pending.graph_complete = true;
                            pending.graph_failure = Some(error.to_string());
                        }
                        Err(HostError::ModuleSyntax(_)) if pending.graph.error_token.is_some() => {
                            pending.graph_complete = true
                        }
                        Err(error) => {
                            if pending.native_failure.is_none() {
                                pending.native_failure=Some(match error{HostError::ScriptLoad(failure)=>failure,error=>crate::script_load_failure::ScriptLoadFailure::Infrastructure(error.to_string())});
                            }
                            pending.retry = Some(Instant::now() + Duration::from_millis(10));
                            registry.borrow_mut().pending.push_back(pending);
                            continue;
                        }
                    }
                }
            }
            match self.complete_dynamic_import(&mut pending) {
                Ok(true) => delivered += 1,
                Ok(false) => {
                    pending.retry = Some(Instant::now() + Duration::from_millis(10));
                    registry.borrow_mut().pending.push_back(pending);
                }
                Err(error) => {
                    pending.retry = Some(Instant::now() + Duration::from_millis(10));
                    registry.borrow_mut().pending.push_back(pending);
                    if self.contexts.contains_key(&previous) {
                        self.active_realm = previous;
                    }
                    return Err(error);
                }
            }
        }
        if self.contexts.contains_key(&previous) {
            self.active_realm = previous;
        }
        Ok(delivered)
    }
    fn complete_dynamic_import(
        &mut self,
        pending: &mut PendingDynamicImport,
    ) -> Result<bool, HostError> {
        if pending.settled {
            return Ok(true);
        }
        let completion_deadline = *pending
            .completion_deadline
            .get_or_insert_with(|| Instant::now() + Duration::from_secs(10));
        if Instant::now() >= completion_deadline
            && pending.loaded.is_none()
            && pending.graph_failure.is_none()
        {
            pending.graph_failure = Some(
                pending
                    .native_failure
                    .take()
                    .map(|failure| failure.to_string())
                    .unwrap_or_else(|| {
                        "dynamic import preparation publication deadline exceeded".into()
                    }),
            );
        }
        // Once evaluation/rejection exists, its retained outcome owns settlement.
        // A publication retry deadline must not orphan the promise or replace a
        // module's original exception/namespace with a scheduling error.
        if self.isolate.get_slot::<ModuleSources>().is_none() {
            self.isolate
                .set_slot(ModuleSources(Arc::new(BTreeMap::new())));
        }
        if let ModuleGraphBackend::Sources(sources, _, _) = &pending.backend {
            self.isolate.set_slot(ModuleSources(sources.clone()));
        }
        let context = self
            .contexts
            .get(&pending.environment.realm)
            .cloned()
            .ok_or_else(|| HostError::Execution("import realm retired".into()))?;
        let deadline = ExecutionDeadline::arm(&self.isolate, self.execution_wall_limit);
        v8::scope!(let scope,&mut self.isolate);
        let context = v8::Local::new(scope, &context);
        let scope = &mut v8::ContextScope::new(scope, context);
        if current_environment(scope).ok() != Some(pending.environment) {
            return Ok(true);
        }
        let resolver = v8::Local::new(scope, &pending.resolver);
        if let Some(error) = &pending.graph_failure {
            let message = v8::String::new(scope, error)
                .ok_or_else(|| HostError::Execution("import rejection allocation".into()))?;
            let error = v8::Exception::type_error(scope, message);
            if retain_module_rejection(scope, resolver, error).is_none() {
                return Ok(false);
            }
            pending.settled = true;
        } else {
            if pending.loaded.is_none() {
                let options = v8::Local::new(scope, &pending.options);
                let resource = v8::Local::new(scope, &pending.resource);
                let specifier = v8::Local::new(scope, &pending.specifier);
                let attributes = v8::Local::new(scope, &pending.attributes);
                let root = specifier.to_rust_string_lossy(scope);
                context.set_slot(Rc::new(PreparedImportLink {
                    root: RefCell::new(Some(root)),
                    error: pending.graph.error_token,
                }));
                let loaded = load_dynamic_import_prepared(
                    scope,
                    options,
                    resource,
                    specifier,
                    pending.phase,
                    attributes,
                );
                context.remove_slot::<PreparedImportLink>();
                pending.loaded = loaded;
            }
            if let Some(loaded) = &pending.loaded {
                #[cfg(test)]
                if FAIL_IMPORT_SETTLEMENT.with(|failure| failure.replace(false)) {
                    return Ok(false);
                }
                pending.settled = match loaded {
                    PreparedModuleImport::Evaluation { namespace, promise } => {
                        let namespace = v8::Local::new(scope, namespace);
                        let promise = v8::Local::new(scope, promise);
                        retain_module_settlement(scope, promise, resolver, namespace).is_some()
                    }
                    PreparedModuleImport::NativeFailure(failure) => {
                        let failure = failure.clone();
                        pending.loaded = None;
                        match failure {
                            crate::script_load_failure::ScriptLoadFailure::Cancelled
                            | crate::script_load_failure::ScriptLoadFailure::Capacity(_) => {
                                pending.graph_failure = Some(failure.to_string());
                                return Ok(false);
                            }
                            _ => {
                                if pending.native_failure.is_none() {
                                    pending.native_failure = Some(failure);
                                }
                                return Ok(false);
                            }
                        }
                    }
                    PreparedModuleImport::Rejection(error) => {
                        let error = v8::Local::new(scope, error);
                        retain_module_rejection(scope, resolver, error).is_some()
                    }
                };
            }
        }
        queue_mutation_observer_microtask(scope);
        scope.perform_microtask_checkpoint();
        if deadline.expired() || scope.is_execution_terminating() {
            scope.cancel_terminate_execution();
            reset_mutation_observer_microtask_after_termination(scope);
            return Err(HostError::LimitExceeded {
                resource: "dynamic import execution wall time".into(),
            });
        }
        Ok(pending.settled)
    }
}
#[cfg(test)]
thread_local! {static FAIL_IMPORT_SETTLEMENT:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}

#[cfg(test)]
mod dynamic_import_continuation_tests {
    use super::*;
    // Module continuations are owned by an admitted Document, not a bare V8 realm.
    pub(super) fn install_document(host: &mut RawV8Host) {
        let generation = host
            .realm_descriptor(host.active_realm())
            .unwrap()
            .document_generation;
        host.install_dom(
            Rc::new(RefCell::new(
                oryn_dom::html::parse("<!doctype html><body></body>")
                    .into_runtime()
                    .unwrap(),
            )),
            generation,
        )
        .unwrap();
        host.commit_location("https://example.test/").unwrap();
    }
    fn host() -> RawV8Host {
        let mut host = RawV8Host::new_with_base_url_and_heap_limit(
            Url::parse("https://example.test/").unwrap(),
            256 * 1024 * 1024,
        );
        install_document(&mut host);
        host
    }
    pub(super) fn settle(host: &mut RawV8Host) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while host.has_pending_dynamic_imports(false) {
            host.drain_async_fetches(false).unwrap();
            assert!(
                Instant::now() < deadline,
                "import continuation did not settle"
            );
            std::thread::yield_now();
        }
    }
    fn media_trace_violation(url: &str) -> crate::csp::CspViolation {
        let policy = crate::csp::CspPolicySet::from_headers(
            "https://example.test/",
            &[("Content-Security-Policy".into(), "media-src 'none'".into())],
        );
        policy
            .authorize(&crate::csp::CspRequest {
                destination: crate::csp::CspDestination::Media,
                url: url.into(),
                nonce: None,
                inline_source: None,
                parser_inserted: false,
                initiated_by_trusted_script: false,
            })
            .violations
            .into_iter()
            .next()
            .unwrap()
    }
    #[test]
    fn native_csp_trace_bounds_storage_and_reports_omission_once() {
        let mut trace = NativeCspTrace::default();
        let violation = media_trace_violation("https://example.test/movie.mp4");
        for _ in 0..NATIVE_CSP_TRACE_COUNT + 3 {
            trace.record(&violation);
        }
        assert_eq!(trace.entries.len(), NATIVE_CSP_TRACE_COUNT);
        let entries = trace.take();
        assert_eq!(entries.len(), NATIVE_CSP_TRACE_COUNT + 1);
        assert_eq!(entries[0].kind, "native_csp_violation");
        assert_eq!(entries[0].detail["effective_directive"], "media-src");
        assert_eq!(entries.last().unwrap().detail["omitted"], 3);
        assert!(trace.take().is_empty());
        let mut oversized = violation.clone();
        oversized.blocked_uri = "x".repeat(NATIVE_CSP_TRACE_BYTES + 1);
        trace.record(&oversized);
        assert!(trace.entries.is_empty());
        assert_eq!(trace.bytes, 0);
        assert_eq!(trace.take()[0].detail["omitted"], 1);
    }
    #[test]
    fn native_csp_trace_keeps_dom_delivery_and_isolates_root_document_authority() {
        let mut host = host();
        let first = host.active_realm();
        host.execute("globalThis.cspEvents=[];addEventListener('securitypolicyviolation',event=>cspEvents.push(event.effectiveDirective))").unwrap();
        let producer = host.script_report_queue().unwrap();
        producer
            .push(vec![media_trace_violation(
                "https://example.test/first.mp4",
            )])
            .unwrap();
        host.drain_scheduled_timers(false).unwrap();
        let second = host.create_realm("https://second.test/", 1, None).unwrap();
        host.activate_realm(second).unwrap();
        install_document(&mut host);
        assert!(
            host.take_native_csp_trace_entries().is_empty(),
            "another root cannot drain the first root"
        );
        host.activate_realm(first).unwrap();
        host.drain_scheduled_timers(false).unwrap();
        assert_eq!(
            host.evaluate_string("cspEvents.join(',')").unwrap(),
            "media-src"
        );
        let entries = host.take_native_csp_trace_entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].detail["blocked_uri"],
            "https://example.test/first.mp4"
        );
        assert!(host.take_native_csp_trace_entries().is_empty());
        producer
            .push(vec![media_trace_violation(
                "https://example.test/stale.mp4",
            )])
            .unwrap();
        host.drain_scheduled_timers(false).unwrap();
        host.execute("document.open();document.write('<body>same document');document.close()")
            .unwrap();
        assert_eq!(
            host.take_native_csp_trace_entries().len(),
            1,
            "document.open retains this Document's accepted report"
        );
        host.script_report_queue()
            .unwrap()
            .push(vec![media_trace_violation(
                "https://example.test/replaced.mp4",
            )])
            .unwrap();
        host.drain_scheduled_timers(false).unwrap();
        host.install_dom(
            Rc::new(RefCell::new(
                oryn_dom::html::parse("<body>replacement")
                    .into_runtime()
                    .unwrap(),
            )),
            2,
        )
        .unwrap();
        assert!(
            host.take_native_csp_trace_entries().is_empty(),
            "replacement native Document cannot inherit unread trace"
        );
    }
    #[test]
    fn native_csp_trace_accepts_contextless_scope_and_restores_entered_context() {
        let mut host = host();
        let first = host.active_realm();
        let first_context = host.contexts.get(&first).unwrap().clone();
        let second = host.create_realm("https://second.test/", 1, None).unwrap();
        host.activate_realm(second).unwrap();
        install_document(&mut host);
        let second_context = host.contexts.get(&second).unwrap().clone();
        {
            v8::scope!(let handles, &mut host.isolate);
            let first_context = v8::Local::new(handles, &first_context);
            let second_context = v8::Local::new(handles, &second_context);
            // This has the same PinScope<(), ...> marker as the report drain.
            record_native_csp_trace(
                handles,
                first_context,
                &media_trace_violation("https://example.test/contextless.mp4"),
            );
            let scope = &mut v8::ContextScope::new(handles, second_context);
            assert!(scope.get_current_context() == second_context);
            record_native_csp_trace(
                scope,
                first_context,
                &media_trace_violation("https://example.test/entered.mp4"),
            );
            assert!(
                scope.get_current_context() == second_context,
                "recording another root must restore the incumbent context"
            );
        }
        assert_eq!(host.active_realm(), second);
        assert!(
            host.take_native_csp_trace_entries().is_empty(),
            "the incumbent root cannot acquire another root's trace"
        );
        host.activate_realm(first).unwrap();
        let entries = host.take_native_csp_trace_entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries[0].detail["blocked_uri"],
            "https://example.test/contextless.mp4"
        );
        assert_eq!(
            entries[1].detail["blocked_uri"],
            "https://example.test/entered.mp4"
        );
        assert!(host.take_native_csp_trace_entries().is_empty());
    }
    #[test]
    fn background_csp_reports_preserve_each_root_and_deliver_author_callbacks_once() {
        let mut host = host();
        let first = host.active_realm();
        let listener = "globalThis.cspEvents=[];addEventListener('securitypolicyviolation',event=>{cspEvents.push(event.effectiveDirective+'|'+event.blockedURI);document.body.setAttribute('data-csp-count',String(cspEvents.length))})";
        host.execute(listener).unwrap();
        let first_reports = host.script_report_queue().unwrap();
        let second = host.create_realm("https://second.test/", 1, None).unwrap();
        host.activate_realm(second).unwrap();
        install_document(&mut host);
        host.execute(listener).unwrap();
        let second_reports = host.script_report_queue().unwrap();
        first_reports
            .push(vec![media_trace_violation(
                "https://example.test/first-pending.mp4",
            )])
            .unwrap();
        second_reports
            .push(vec![media_trace_violation(
                "https://example.test/second-pending.mp4",
            )])
            .unwrap();

        host.drain_background_script_reports().unwrap();
        host.drain_background_script_reports().unwrap();
        assert_eq!(host.active_realm(), second);
        assert!(!first_reports.pending());
        assert!(!second_reports.pending());
        for (realm, uri) in [
            (first, "https://example.test/first-pending.mp4"),
            (second, "https://example.test/second-pending.mp4"),
        ] {
            assert!(
                host.binding_states
                    .get(&realm)
                    .unwrap()
                    .has_pending_native_csp_event(),
                "native tracing must preserve the admitted DOM event"
            );
            host.activate_realm(realm).unwrap();
            let entries = host.take_native_csp_trace_entries();
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].kind, "native_csp_violation");
            assert_eq!(entries[0].detail["disposition"], "enforce");
            assert_eq!(entries[0].detail["effective_directive"], "media-src");
            assert_eq!(entries[0].detail["blocked_uri"], uri);
            assert!(host.take_native_csp_trace_entries().is_empty());
        }
        host.drain_scheduled_timers(false).unwrap();
        host.drain_scheduled_timers(false).unwrap();
        for (realm, expected) in [
            (first, "media-src|https://example.test/first-pending.mp4"),
            (second, "media-src|https://example.test/second-pending.mp4"),
        ] {
            host.activate_realm(realm).unwrap();
            assert_eq!(
                host.evaluate_string("cspEvents.join(',')").unwrap(),
                expected
            );
            assert_eq!(
                host.evaluate_string("document.body.getAttribute('data-csp-count')")
                    .unwrap(),
                "1"
            );
            assert!(
                !host
                    .binding_states
                    .get(&realm)
                    .unwrap()
                    .has_pending_native_csp_event()
            );
            assert!(
                host.take_native_csp_trace_entries().is_empty(),
                "DOM delivery must not record an admitted violation twice"
            );
        }
    }
    #[test]
    fn completed_native_resource_timing_is_visible_at_first_authored_instruction() {
        let mut host = host();
        host.execute("document.body.innerHTML='<p>root</p>'")
            .unwrap();
        let producer = host.script_report_queue().unwrap();
        let active = producer.time_resource("https://example.test/resource.js", "script");
        host.execute("if(performance.getEntriesByType('resource').length)throw Error('unfinished resource published')").unwrap();
        drop(active);
        host.execute("const entries=performance.getEntriesByName('https://example.test/resource.js');if(entries.length!==1||entries[0].initiatorType!=='script'||entries[0].responseEnd<entries[0].startTime)throw Error('native timing not published before execution')").unwrap();
        host.execute("if(performance.getEntriesByType('resource').length!==1)throw Error('duplicate native timing')").unwrap();
    }
    #[test]
    fn standalone_import_defers_graph_evaluation_and_shares_namespace() {
        let mut host = host();
        host.set_module_sources(&BTreeMap::from([
            ("https://example.test/root.js".into(),"import {value} from './child.js';globalThis.evaluations=(globalThis.evaluations||0)+1;export {value}".into()),
            ("https://example.test/child.js".into(),"export const value=42".into()),
        ]));
        host.execute("globalThis.done=false;Promise.all([import('https://example.test/root.js'),import('https://example.test/root.js')]).then(([a,b])=>{globalThis.same=a===b;globalThis.value=a.value;done=true})").unwrap();
        assert_eq!(
            host.evaluate_string("typeof evaluations+'|'+done").unwrap(),
            "undefined|false"
        );
        settle(&mut host);
        assert_eq!(
            host.evaluate_string("same+'|'+value+'|'+evaluations+'|'+done")
                .unwrap(),
            "true|42|1|true"
        );
    }
    #[test]
    fn deferred_import_resolves_without_body_until_native_namespace_access() {
        let mut host = host();
        host.set_module_sources(&BTreeMap::from([(
            "https://example.test/deferred.js".into(),
            "globalThis.runs=(globalThis.runs||0)+1;export const value=42;".into(),
        )]));
        host.execute("import.defer('https://example.test/deferred.js').then(ns=>globalThis.ns=ns)")
            .unwrap();
        settle(&mut host);
        assert_eq!(host.evaluate_string("typeof runs").unwrap(), "undefined");
        assert_eq!(
            host.evaluate_string("String(ns.value)+'|'+runs").unwrap(),
            "42|1"
        );
        host.execute("import('https://example.test/deferred.js').then(ns=>globalThis.normal=ns)")
            .unwrap();
        settle(&mut host);
        assert_eq!(
            host.evaluate_string("String(normal.value)+'|'+runs")
                .unwrap(),
            "42|1"
        );
    }
    #[test]
    fn deferred_import_eagerly_evaluates_tla_dependencies_only() {
        let mut host = host();
        host.set_module_sources(&BTreeMap::from([
            (
                "https://example.test/entry.js".into(),
                "import {value} from './async.js';globalThis.entryRan=true;export {value};".into(),
            ),
            (
                "https://example.test/async.js".into(),
                "globalThis.dependencyRan=true;await Promise.resolve();export const value='ready';"
                    .into(),
            ),
        ]));
        host.execute("import.defer('https://example.test/entry.js').then(ns=>globalThis.ns=ns)")
            .unwrap();
        settle(&mut host);
        assert_eq!(
            host.evaluate_string("dependencyRan+'|'+typeof entryRan")
                .unwrap(),
            "true|undefined"
        );
        assert_eq!(
            host.evaluate_string("ns.value+'|'+entryRan").unwrap(),
            "ready|true"
        );
    }
    #[test]
    fn deferred_cyclic_tla_remains_pending_without_eager_entry_or_error() {
        // The ordinary static edge makes evaluation of a wait for async b.
        // An ordinary deferred namespace read instead rejects while b is
        // async-not-ready; a symbol-like then read reaches the uninitialized binding.
        let mut host = host();
        host.set_module_sources(&BTreeMap::from([
            (
                "https://example.test/a.js".into(),
                "import {value} from './b.js';globalThis.aRan=true;export {value};".into(),
            ),
            (
                "https://example.test/b.js".into(),
                "globalThis.bRan=true;await import('./a.js');export const value=42;".into(),
            ),
        ]));
        host.execute("globalThis.result='pending';import.defer('https://example.test/a.js').then(()=>result='fulfilled',()=>result='rejected')").unwrap();
        settle(&mut host);
        for _ in 0..10 {
            host.run_microtask_checkpoint().unwrap();
            host.drain_async_fetches(false).unwrap();
        }
        assert_eq!(
            host.evaluate_string("bRan+'|'+typeof aRan+'|'+result")
                .unwrap(),
            "true|undefined|pending"
        );
        assert_eq!(
            host.isolate
                .get_slot::<DeferredModuleEvaluations>()
                .unwrap()
                .0
                .len(),
            1
        );
    }

    #[test]
    fn deferred_namespace_cycle_reports_tdz_without_replaying_module_bodies() {
        // The symbol-like `then` key does not trigger synchronous evaluation
        // of a deferred namespace. It reaches GetExport while the binding is
        // uninitialized, unlike ordinary b.value on an async-not-ready module.
        let mut host = host();
        host.set_module_sources(&BTreeMap::from([
          ("https://example.test/a.js".into(),"import defer * as b from './b.js';globalThis.aRuns=(globalThis.aRuns||0)+1;export const value=b.then;".into()),
          ("https://example.test/b.js".into(),"globalThis.bRuns=(globalThis.bRuns||0)+1;await import('./a.js');export const then=42;".into())]));
        host.execute("globalThis.result='pending';import.defer('https://example.test/a.js').then(()=>result='fulfilled',e=>result=e.name)").unwrap();
        settle(&mut host);
        for _ in 0..10 {
            host.run_microtask_checkpoint().unwrap();
            host.drain_async_fetches(false).unwrap();
        }
        assert_eq!(
            host.evaluate_string("aRuns+'|'+bRuns+'|'+result").unwrap(),
            "1|1|ReferenceError"
        );
        host.execute("import('./a.js').catch(e=>globalThis.repeated=e.name)")
            .unwrap();
        settle(&mut host);
        assert_eq!(
            host.evaluate_string("aRuns+'|'+bRuns+'|'+repeated")
                .unwrap(),
            "1|1|ReferenceError"
        );
    }

    #[test]
    fn deferred_namespace_async_not_ready_preserves_type_error_without_replaying_bodies() {
        let mut host = host();
        host.set_module_sources(&BTreeMap::from([
          ("https://example.test/a.js".into(),"import defer * as b from './b.js';globalThis.aRuns=(globalThis.aRuns||0)+1;export const value=b.value;".into()),
          ("https://example.test/b.js".into(),"globalThis.bRuns=(globalThis.bRuns||0)+1;await import('./a.js');export const value=42;".into())]));
        host.execute("globalThis.result='pending';import.defer('https://example.test/a.js').then(()=>result='fulfilled',e=>result=e.name)").unwrap();
        settle(&mut host);
        for _ in 0..10 {
            host.run_microtask_checkpoint().unwrap();
            host.drain_async_fetches(false).unwrap();
        }
        assert_eq!(
            host.evaluate_string("aRuns+'|'+bRuns+'|'+result").unwrap(),
            "1|1|TypeError"
        );
        host.execute("import('./a.js').catch(e=>globalThis.repeated=e.name)")
            .unwrap();
        settle(&mut host);
        assert_eq!(
            host.evaluate_string("aRuns+'|'+bRuns+'|'+repeated")
                .unwrap(),
            "1|1|TypeError"
        );
    }

    fn reach_retained_settlement(host: &mut RawV8Host) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            host.drain_async_fetches(false).unwrap();
            if host
                .isolate
                .get_slot::<Rc<RefCell<DynamicImports>>>()
                .unwrap()
                .borrow()
                .pending
                .front()
                .is_some_and(|pending| pending.loaded.is_some())
            {
                break;
            }
            assert!(Instant::now() < deadline, "retained settlement not reached");
            std::thread::yield_now();
        }
    }
    #[test]
    fn import_settlement_retry_keeps_body_promise_and_original_namespace() {
        let mut host = host();
        host.set_module_sources(&BTreeMap::from([(
            "https://example.test/value.js".into(),
            "globalThis.runs=(globalThis.runs||0)+1;export const value=42;".into(),
        )]));
        host.execute("globalThis.done=false;import('https://example.test/value.js').then(m=>{globalThis.value=m.value;done=true})").unwrap();
        FAIL_IMPORT_SETTLEMENT.with(|failure| failure.set(true));
        reach_retained_settlement(&mut host);
        let registry = host
            .isolate
            .get_slot::<Rc<RefCell<DynamicImports>>>()
            .unwrap()
            .clone();
        registry.borrow_mut().pending.front_mut().unwrap().retry =
            Some(Instant::now() + Duration::from_secs(1));
        assert!(registry.borrow().pending.front().unwrap().loaded.is_some());
        assert_eq!(host.evaluate_string("runs+'|'+done").unwrap(), "1|false");
        assert!(!host.has_pending_dynamic_imports(true));
        registry.borrow_mut().pending.front_mut().unwrap().retry = None;
        settle(&mut host);
        assert_eq!(
            host.evaluate_string("runs+'|'+done+'|'+value").unwrap(),
            "1|true|42"
        );
        assert_eq!(registry.borrow().admitted.get(), 0);
    }
    #[test]
    fn expired_import_graph_rejects_live_promise_and_preserves_successors() {
        let mut host = host();
        host.set_module_sources(&BTreeMap::from([
            (
                "https://example.test/expired.js".into(),
                "globalThis.expiredRan=true;export {}".into(),
            ),
            (
                "https://example.test/next.js".into(),
                "export const value=42".into(),
            ),
        ]));
        host.execute("globalThis.failure='pending';import('./expired.js').catch(e=>failure=e.name);import('./next.js').then(m=>globalThis.answer=m.value)").unwrap();
        let registry = host
            .isolate
            .get_slot::<Rc<RefCell<DynamicImports>>>()
            .unwrap()
            .clone();
        assert!(
            host.dynamic_import_completion_delay().is_some(),
            "waiting graph owns a deadline wakeup"
        );
        registry.borrow_mut().pending.front_mut().unwrap().deadline =
            Instant::now() - Duration::from_secs(1);
        assert_eq!(host.dynamic_import_completion_delay(), Some(Duration::ZERO));
        settle(&mut host);
        assert_eq!(
            host.evaluate_string("failure+'|'+typeof expiredRan+'|'+answer")
                .unwrap(),
            "TypeError|undefined|42"
        );
        assert_eq!(registry.borrow().admitted.get(), 0);
    }
    #[test]
    fn expired_publication_retry_keeps_evaluated_namespace_and_settles_once() {
        let mut host = host();
        host.set_module_sources(&BTreeMap::from([(
            "https://example.test/value.js".into(),
            "globalThis.runs=(globalThis.runs||0)+1;export const value=42".into(),
        )]));
        host.execute("globalThis.fulfilled=0;globalThis.rejected=0;import('./value.js').then(m=>{fulfilled++;globalThis.answer=m.value},()=>rejected++)").unwrap();
        FAIL_IMPORT_SETTLEMENT.with(|failure| failure.set(true));
        reach_retained_settlement(&mut host);
        let registry = host
            .isolate
            .get_slot::<Rc<RefCell<DynamicImports>>>()
            .unwrap()
            .clone();
        {
            let mut registry = registry.borrow_mut();
            let pending = registry.pending.front_mut().unwrap();
            pending.retry = None;
            pending.completion_deadline = Some(Instant::now() - Duration::from_secs(1));
        }
        settle(&mut host);
        assert_eq!(
            host.evaluate_string("runs+'|'+fulfilled+'|'+rejected+'|'+answer")
                .unwrap(),
            "1|1|0|42"
        );
        assert_eq!(registry.borrow().admitted.get(), 0);
    }
    #[test]
    fn import_original_parse_exception_survives_settlement_retry() {
        let mut host = host();
        host.set_module_sources(&BTreeMap::from([(
            "https://example.test/bad.js".into(),
            "export const broken = ;".into(),
        )]));
        host.execute(
            "globalThis.errors=[];import('https://example.test/bad.js').catch(e=>errors.push(e))",
        )
        .unwrap();
        FAIL_IMPORT_SETTLEMENT.with(|failure| failure.set(true));
        reach_retained_settlement(&mut host);
        let registry = host
            .isolate
            .get_slot::<Rc<RefCell<DynamicImports>>>()
            .unwrap()
            .clone();
        registry.borrow_mut().pending.front_mut().unwrap().retry = None;
        settle(&mut host);
        host.execute("import('https://example.test/bad.js').catch(e=>errors.push(e))")
            .unwrap();
        settle(&mut host);
        assert_eq!(
            host.evaluate_string(
                "String(errors.length===2&&errors[0] instanceof SyntaxError&&errors[0]===errors[1])"
            )
            .unwrap(),
            "true"
        );
    }
    #[test]
    fn background_script_reports_keep_document_identity_and_survive_open_and_producer_drop() {
        let mut host = host();
        host.execute("document.body.innerHTML='<p>root</p>'")
            .unwrap();
        host.execute("globalThis.violations=0;document.addEventListener('securitypolicyviolation',()=>violations++)").unwrap();
        let policy = crate::csp::CspPolicySet::from_headers(
            "https://example.test/",
            &[("Content-Security-Policy".into(), "script-src 'none'".into())],
        );
        let violation = policy
            .authorize(&crate::csp::CspRequest {
                destination: crate::csp::CspDestination::Script,
                url: "https://example.test/blocked.js".into(),
                nonce: None,
                inline_source: None,
                parser_inserted: true,
                initiated_by_trusted_script: false,
            })
            .violations
            .remove(0);
        let producer = host.script_report_queue().unwrap();
        producer.push(vec![violation.clone()]).unwrap();
        drop(producer);
        assert_eq!(host.evaluate_string("String(violations)").unwrap(), "0");
        host.drain_scheduled_timers(false).unwrap();
        assert_eq!(host.evaluate_string("String(violations)").unwrap(), "1");
        host.drain_scheduled_timers(false).unwrap();
        assert_eq!(host.evaluate_string("String(violations)").unwrap(), "1");
        let obsolete = host.script_report_queue().unwrap();
        host.execute("document.open();document.write('<p>new</p>');document.close();document.addEventListener('securitypolicyviolation',()=>violations++)").unwrap();
        obsolete.push(vec![violation]).unwrap();
        host.drain_scheduled_timers(false).unwrap();
        assert_eq!(host.evaluate_string("String(violations)").unwrap(), "2");
    }

    #[test]
    fn native_policy_preparations_share_immutable_payload_until_meta_changes() {
        let policy = RefCell::new(crate::csp::CspPolicySet::new("https://example.test/"));
        let cached = RefCell::new(None);
        let first = capture_standalone_script_policy(&policy, &cached).unwrap();
        let second = capture_standalone_script_policy(&policy, &cached).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        policy.borrow_mut().install_meta("script-src 'none'");
        let later = capture_standalone_script_policy(&policy, &cached).unwrap();
        assert!(!Arc::ptr_eq(&first, &later));
        let request = crate::csp::CspRequest {
            destination: crate::csp::CspDestination::Script,
            url: "https://example.test/source.js".into(),
            nonce: None,
            inline_source: None,
            parser_inserted: true,
            initiated_by_trusted_script: false,
        };
        assert!(
            first
                .native::<crate::csp::CspPolicySet>()
                .unwrap()
                .authorize(&request)
                .allowed
        );
        assert!(
            !later
                .native::<crate::csp::CspPolicySet>()
                .unwrap()
                .authorize(&request)
                .allowed
        );
        assert!(
            oryn_js::v8_bindings::ScriptPolicySnapshot::new("x".repeat(4 * 1024 * 1024 + 1), ())
                .is_err()
        );
    }

    #[test]
    fn retained_import_source_survives_source_map_replacement_without_copying() {
        let mut host = host();
        host.set_module_sources(&BTreeMap::from([(
            "https://example.test/value.js".into(),
            "export const value=1".into(),
        )]));
        host.execute("import('https://example.test/value.js').then(m=>globalThis.answer=m.value)")
            .unwrap();
        let snapshot = host.isolate.get_slot::<ModuleSources>().unwrap().0.clone();
        let pending = host
            .isolate
            .get_slot::<Rc<RefCell<DynamicImports>>>()
            .unwrap()
            .borrow();
        let ModuleGraphBackend::Sources(captured, _, _) = &pending.pending.front().unwrap().backend
        else {
            panic!("explicit source backend")
        };
        assert!(Arc::ptr_eq(captured, &snapshot));
        drop(pending);
        host.set_module_sources(&BTreeMap::from([(
            "https://example.test/value.js".into(),
            "export const value=2".into(),
        )]));
        settle(&mut host);
        assert_eq!(host.evaluate_string("String(answer)").unwrap(), "1");
    }
    #[test]
    fn syntax_failure_rejects_import_without_running_module_body() {
        let mut host = host();
        host.set_module_sources(&BTreeMap::from([(
            "https://example.test/bad.js".into(),
            "globalThis.badRan=true;export const =".into(),
        )]));
        host.execute("import('https://example.test/bad.js').catch(e=>globalThis.failure=e.name)")
            .unwrap();
        settle(&mut host);
        assert_eq!(
            host.evaluate_string("failure+'|'+typeof badRan").unwrap(),
            "SyntaxError|undefined"
        );
    }
    #[test]
    fn retired_environment_drops_pending_resolver_and_graph_admission() {
        let mut host = host();
        host.set_module_sources(&BTreeMap::from([(
            "https://example.test/value.js".into(),
            "globalThis.mustNotRun=true;export {}".into(),
        )]));
        host.execute("import('https://example.test/value.js')")
            .unwrap();
        let registry = host
            .isolate
            .get_slot::<Rc<RefCell<DynamicImports>>>()
            .unwrap()
            .clone();
        let owner = registry.borrow().pending.front().unwrap().environment;
        assert_eq!(registry.borrow().admitted.get(), 1);
        retire_environment(&mut host.isolate, owner.id);
        assert!(registry.borrow().pending.is_empty());
        assert_eq!(registry.borrow().admitted.get(), 0);
    }
    #[test]
    fn direct_module_admission_retires_before_fetch_and_failure_keeps_successors() {
        struct NoFetch;
        impl crate::network::NetworkBroker for NoFetch {
            fn send(
                &self,
                _request: crate::network::NetworkRequest,
            ) -> Result<crate::network::NetworkResponse, crate::network::NetworkError> {
                panic!("unexpected direct module network request")
            }
        }
        let make = || {
            let mut host = RawV8Host::new_with_network(
                SharedNetworkBroker::new(NoFetch),
                Url::parse("https://example.test/").unwrap(),
            );
            install_document(&mut host);
            host
        };
        let mut host = make();
        let slots = Arc::new(std::sync::atomic::AtomicUsize::new(8));
        host.set_dynamic_import_limits(
            host.active_realm(),
            Default::default(),
            slots,
            Default::default(),
        )
        .unwrap();
        host.execute_module(
            "https://example.test/obsolete.js",
            "import './not-admitted.js';",
            &BTreeMap::new(),
        )
        .unwrap();
        let registry = host
            .isolate
            .get_slot::<Rc<RefCell<DynamicImports>>>()
            .unwrap()
            .clone();
        let owner = registry.borrow().direct.front().unwrap().environment;
        assert_eq!(registry.borrow().admitted.get(), 1);
        assert!(registry.borrow().direct.front().is_some());
        retire_environment(&mut host.isolate, owner.id);
        assert!(registry.borrow().direct.is_empty());
        assert_eq!(registry.borrow().admitted.get(), 0);
        assert_eq!(
            registry
                .borrow()
                .bytes
                .load(std::sync::atomic::Ordering::Acquire),
            0
        );
        drop(host);
        let mut host = make();
        let slots = Arc::new(std::sync::atomic::AtomicUsize::new(8));
        host.set_dynamic_import_limits(
            host.active_realm(),
            Default::default(),
            slots,
            Default::default(),
        )
        .unwrap();
        host.execute_module(
            "https://example.test/bad.js",
            "import './held.js';throw new Error('direct failure');",
            &BTreeMap::new(),
        )
        .unwrap();
        host.execute_module(
            "https://example.test/good.js",
            "import './held.js';globalThis.directSuccess=42;",
            &BTreeMap::new(),
        )
        .unwrap();
        host.retain_verified_module_fetch(
            "https://example.test/held.js",
            "https://example.test/held.js",
            "export {};",
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if host.drain_async_fetches(false).is_err() {
                break;
            }
            assert!(
                host.has_pending_dynamic_imports(false),
                "failure must be reported before the successor completes"
            );
            assert!(
                Instant::now() < deadline,
                "direct module failure not reached"
            );
            std::thread::yield_now();
        }
        assert!(host.has_pending_dynamic_imports(false));
        settle(&mut host);
        assert_eq!(host.evaluate_string("String(directSuccess)").unwrap(), "42");
        assert_eq!(
            host.isolate
                .get_slot::<Rc<RefCell<DynamicImports>>>()
                .unwrap()
                .borrow()
                .admitted
                .get(),
            0
        );
    }

    #[test]
    fn held_import_headers_yield_to_author_tasks_and_duplicate_fetch_joins() {
        struct Held {
            entered: std::sync::mpsc::Sender<()>,
            release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
            calls: Arc<std::sync::atomic::AtomicUsize>,
        }
        impl crate::network::NetworkBroker for Held {
            fn send(
                &self,
                request: crate::network::NetworkRequest,
            ) -> Result<crate::network::NetworkResponse, crate::network::NetworkError> {
                self.calls.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                self.entered.send(()).unwrap();
                self.release.lock().unwrap().recv().unwrap();
                Ok(crate::network::NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![("content-type".into(), "text/javascript".into())],
                    body: b"export const value=42".to_vec(),
                })
            }
        }
        let (entered, observed) = std::sync::mpsc::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut host = RawV8Host::new_with_network(
            SharedNetworkBroker::new(Held {
                entered,
                release: std::sync::Mutex::new(blocked),
                calls: calls.clone(),
            }),
            Url::parse("https://example.test/").unwrap(),
        );
        install_document(&mut host);
        host.execute_module(
            "https://example.test/direct.js",
            "import './held.js';globalThis.directReady=true",
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(
            host.evaluate_string("typeof directReady").unwrap(),
            "undefined"
        );
        host.execute("globalThis.ready=false;Promise.all([import('./held.js'),import('./held.js')]).then(([a,b])=>{globalThis.same=a===b;ready=true})").unwrap();
        assert_eq!(host.evaluate_string("String(ready)").unwrap(), "false");
        host.drain_async_fetches(false).unwrap();
        observed.recv_timeout(Duration::from_secs(5)).unwrap();
        host.execute("globalThis.unrelated=42").unwrap();
        host.drain_async_fetches(false).unwrap();
        assert_eq!(
            host.evaluate_string("unrelated+'|'+ready").unwrap(),
            "42|false"
        );
        release.send(()).unwrap();
        settle(&mut host);
        assert_eq!(host.evaluate_string("String(directReady)").unwrap(), "true");
        assert_eq!(host.evaluate_string("same+'|'+ready").unwrap(), "true|true");
        assert_eq!(calls.load(std::sync::atomic::Ordering::Acquire), 1);
    }
    #[test]
    fn document_open_preserves_import_owner_and_selected_graph_budget() {
        let mut host = host();
        let realm = host.active_realm();
        host.execute("document.body.innerHTML='<p>original</p>'")
            .unwrap();
        let slots = Arc::new(std::sync::atomic::AtomicUsize::new(8));
        host.set_dynamic_import_limits(
            realm,
            Default::default(),
            slots.clone(),
            Default::default(),
        )
        .unwrap();
        host.set_module_sources(&BTreeMap::from([(
            "https://example.test/value.js".into(),
            "export const value=42".into(),
        )]));
        host.execute("import('https://example.test/value.js').then(m=>globalThis.answer=m.value)")
            .unwrap();
        let epoch = host.document_authority_epoch(realm);
        let stream = host.document_stream_epoch(realm);
        host.execute("document.open();document.write('<p>replacement</p>');document.close()")
            .unwrap();
        assert_eq!(host.document_authority_epoch(realm), epoch);
        assert_ne!(host.document_stream_epoch(realm), stream);
        host.drain_async_fetches(false).unwrap();
        assert_eq!(host.evaluate_string("typeof answer").unwrap(), "undefined");
        slots.store(0, std::sync::atomic::Ordering::Release);
        settle(&mut host);
        assert_eq!(host.evaluate_string("String(answer)").unwrap(), "42");
    }
    #[cfg(unix)]
    #[test]
    fn parked_native_root_keeps_import_unexecuted_until_authority_restore() {
        let context = crate::Browser::in_process_with_config(Default::default()).new_context();
        let page = context.new_page();
        let url = Url::parse("https://example.test/").unwrap();
        context
            .install_document_security(page.page_id(), url.to_string(), &[])
            .unwrap();
        let client = SharedContextClient::direct(context.clone(), page.page_id());
        let mut host = RawV8Host::new_with_network_context_and_heap_limit(
            None,
            Some(url),
            client,
            page.page_id(),
            256 * 1024 * 1024,
        )
        .unwrap();
        let realm = host.active_realm();
        let generation = host.realm_descriptor(realm).unwrap().document_generation;
        host.install_dom(
            Rc::new(RefCell::new(
                oryn_dom::html::parse("<p>root</p>").into_runtime().unwrap(),
            )),
            generation,
        )
        .unwrap();
        host.commit_location("https://example.test/").unwrap();
        host.retain_verified_module_fetch(
            "https://example.test/value.js",
            "https://example.test/value.js",
            "globalThis.executions=(globalThis.executions||0)+1;export const value=42",
        )
        .unwrap();
        host.execute("import('./value.js').then(m=>globalThis.answer=m.value)")
            .unwrap();
        host.park_document_authority(realm).unwrap();
        assert_eq!(host.poll_dynamic_imports().unwrap(), 0);
        assert_eq!(
            host.isolate
                .get_slot::<Rc<RefCell<DynamicImports>>>()
                .unwrap()
                .borrow()
                .pending
                .len(),
            1
        );
        host.restore_document_authority(realm).unwrap();
        settle(&mut host);
        assert_eq!(
            host.evaluate_string("answer+'|'+executions").unwrap(),
            "42|1"
        );
    }

    #[test]
    fn typed_import_cache_is_separate_and_fetch_failure_is_sticky() {
        struct JsonOnly(Arc<std::sync::atomic::AtomicUsize>);
        impl crate::network::NetworkBroker for JsonOnly {
            fn send(
                &self,
                request: crate::network::NetworkRequest,
            ) -> Result<crate::network::NetworkResponse, crate::network::NetworkError> {
                self.0.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                Ok(crate::network::NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: 200,
                    headers: vec![("content-type".into(), "application/json".into())],
                    body: br#"{"value":42}"#.to_vec(),
                })
            }
        }
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut host = RawV8Host::new_with_network(
            SharedNetworkBroker::new(JsonOnly(calls.clone())),
            Url::parse("https://example.test/").unwrap(),
        );
        install_document(&mut host);
        host.execute(
            "import('./value',{with:{type:'json'}}).then(m=>globalThis.value=m.default.value)",
        )
        .unwrap();
        settle(&mut host);
        host.execute("import('./value').catch(e=>globalThis.failure=e.name)")
            .unwrap();
        settle(&mut host);
        assert_eq!(
            host.evaluate_string("value+'|'+failure").unwrap(),
            "42|TypeError"
        );
        assert_eq!(calls.load(std::sync::atomic::Ordering::Acquire), 2);
        host.execute("import('./value').catch(e=>globalThis.repeatedFailure=e.name)")
            .unwrap();
        settle(&mut host);
        assert_eq!(
            host.evaluate_string("repeatedFailure").unwrap(),
            "TypeError"
        );
        assert_eq!(calls.load(std::sync::atomic::Ordering::Acquire), 2);
    }

    #[test]
    fn captured_import_map_root_is_not_resolved_a_second_time() {
        let mut host = host();
        host.execute("document.body.innerHTML='<p>root</p>'")
            .unwrap();
        host.register_import_map(r#"{"imports":{"named":"https://example.test/value.js","https://example.test/value.js":null}}"#).unwrap();
        host.set_module_sources(&BTreeMap::from([(
            "https://example.test/value.js".into(),
            "export const value=42".into(),
        )]));
        host.execute(
            "import('named').then(m=>globalThis.answer=m.value,e=>globalThis.answer=e.name)",
        )
        .unwrap();
        settle(&mut host);
        assert_eq!(host.evaluate_string("String(answer)").unwrap(), "42");
    }

    #[test]
    fn later_dynamic_import_evaluation_does_not_inherit_script_element_write_counter() {
        let mut host = host();
        host.execute("document.body.innerHTML='<p id=original>root</p>'")
            .unwrap();
        host.set_module_sources(&BTreeMap::from([(
            "https://example.test/write.js".into(),
            "document.write('<p id=written>replacement</p>');export {}".into(),
        )]));
        host.execute("import('https://example.test/write.js')")
            .unwrap();
        settle(&mut host);
        assert_eq!(
            host.evaluate_string("String(!!document.getElementById('written'))")
                .unwrap(),
            "true"
        );
    }
}

fn authorize_standalone_inline_style(
    _state: &V8BindingState,
    policy: &crate::csp::CspPolicySet,
    source: &str,
    nonce: Option<&str>,
    attribute: bool,
    reports: &Arc<crate::executable::module_graph_load::ScriptCspReports>,
) -> Result<bool, String> {
    let decision = policy.authorize(&crate::csp::CspRequest {
        destination: if attribute {
            crate::csp::CspDestination::StyleAttribute
        } else {
            crate::csp::CspDestination::Style
        },
        url: "inline".into(),
        nonce: nonce.map(str::to_owned),
        inline_source: Some(source.into()),
        parser_inserted: true,
        initiated_by_trusted_script: false,
    });
    reports.push_with_policy(policy, decision.violations)?;
    Ok(decision.allowed)
}

fn authorize_standalone_inline_script(
    _state: &V8BindingState,
    policy: &crate::csp::CspPolicySet,
    source: &str,
    nonce: Option<&str>,
    parser_inserted: bool,
    speculation_rules: bool,
    reports: &Arc<crate::executable::module_graph_load::ScriptCspReports>,
) -> Result<bool, String> {
    let request = crate::csp::CspRequest {
        destination: crate::csp::CspDestination::Script,
        url: "inline".into(),
        nonce: nonce.map(str::to_owned),
        inline_source: Some(source.into()),
        parser_inserted,
        initiated_by_trusted_script: !parser_inserted,
    };
    let decision = if speculation_rules {
        policy.authorize_speculation_rules(&request)
    } else {
        policy.authorize(&request)
    };
    reports.push_with_policy(policy, decision.violations)?;
    Ok(decision.allowed
        && !policy
            .sandbox()
            .is_some_and(|sandbox| !sandbox.allow_scripts))
}

fn queue_standalone_csp_violation(
    state: &V8BindingState,
    violation: crate::csp::CspViolation,
) -> Result<(), String> {
    state.queue_native_csp_event(oryn_js::platform_control::CspEvent {
        id: 0,
        document_uri: violation.document_uri,
        referrer: violation.referrer,
        blocked_uri: violation.blocked_uri,
        effective_directive: violation.effective_directive,
        original_policy: violation.original_policy,
        source_file: violation.source_file,
        sample: violation.sample,
        disposition: match violation.disposition {
            crate::csp::CspDisposition::Enforce => "enforce",
            crate::csp::CspDisposition::Report => "report",
        }
        .into(),
        status_code: violation.status_code,
        line_number: violation.line_number,
        column_number: violation.column_number,
    })
}

fn capture_standalone_script_policy(
    policy: &RefCell<crate::csp::CspPolicySet>,
    cached: &RefCell<Option<Arc<oryn_js::v8_bindings::ScriptPolicySnapshot>>>,
) -> Result<Arc<oryn_js::v8_bindings::ScriptPolicySnapshot>, String> {
    struct Writer(Vec<u8>);
    impl std::io::Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.0.len().saturating_add(bytes.len()) > 4 * 1024 * 1024 {
                return Err(std::io::Error::other("script policy serialization limit"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let policy = policy.borrow();
    let mut writer = Writer(Vec::new());
    serde_json::to_writer(&mut writer, &*policy).map_err(|error| error.to_string())?;
    let serialized = String::from_utf8(writer.0).map_err(|error| error.to_string())?;
    if let Some(previous) = cached
        .borrow()
        .as_ref()
        .filter(|previous| previous.as_str() == serialized)
    {
        return Ok(previous.clone());
    }
    let snapshot = oryn_js::v8_bindings::ScriptPolicySnapshot::new(serialized, policy.clone())?;
    *cached.borrow_mut() = Some(snapshot.clone());
    Ok(snapshot)
}

fn module_fetch_backend_in_scope(
    scope: &mut v8::PinScope,
) -> Result<ModuleGraphBackend, HostError> {
    current_environment(scope).map_err(HostError::Execution)?;
    if let Some(agent) = scope.get_slot::<AgentExecution>().cloned() {
        return Ok(ModuleGraphBackend::Worker {
            transport: agent.transport,
            bootstrap: agent.bootstrap,
        });
    }
    let context = scope.get_current_context();
    let binding = context.get_slot::<V8BindingState>();
    #[cfg(unix)]
    if let Some(endpoint) = current_context_endpoint(scope) {
        let binding =
            binding.ok_or_else(|| HostError::Execution("module source binding absent".into()))?;
        let creator = binding
            .native_worker_creator()
            .map_err(|error| HostError::Execution(error.to_string()))?;
        let initiator = binding
            .request_initiator_url()
            .map_err(|error| HostError::Execution(error.to_string()))?;
        return Ok(ModuleGraphBackend::Window(
            endpoint.client,
            endpoint.page_id,
            Some((creator, initiator)),
            binding
                .capture_script_policy()
                .map_err(|error| HostError::Execution(error.to_string()))?,
        ));
    }
    let reports = native_script_report_queue(scope);
    let csp = binding
        .as_ref()
        .map(|binding| binding.capture_local_script_policy())
        .transpose()
        .map_err(|error| HostError::Execution(error.to_string()))?
        .flatten();
    if let Some(fetch) = current_fetch_endpoint(scope) {
        let initiator = binding
            .and_then(|binding| binding.request_initiator_url().ok())
            .unwrap_or_else(|| fetch.base_url.to_string());
        return Ok(ModuleGraphBackend::Broker {
            coep: context
                .get_slot::<crate::standalone_document_policy::DocumentPolicy>()
                .map(|policy| crate::coep_reporting::Policy::new(&initiator, &policy.headers))
                .unwrap_or_default(),
            broker: fetch.broker,
            base: fetch.base_url,
            initiator,
            isolation: context
                .get_slot::<crate::agent_isolation::DocumentIsolationPolicy>()
                .map(|policy| *policy)
                .unwrap_or_default(),
            csp,
            reports,
        });
    }
    Ok(ModuleGraphBackend::Sources(
        scope
            .get_slot::<ModuleSources>()
            .map(|sources| sources.0.clone())
            .unwrap_or_default(),
        csp,
        reports,
    ))
}
