// Exact import Promise settlement custody. Evaluation is observed once and
// forwards directly to the authored import resolver, without a second Promise
// assimilation layer. Callback failure keeps the original namespace/exception.
enum PreparedModuleImport {
    Evaluation {
        namespace: v8::Global<v8::Value>,
        promise: v8::Global<v8::Promise>,
    },
    Rejection(v8::Global<v8::Value>),
    NativeFailure(crate::script_load_failure::ScriptLoadFailure),
}
#[derive(Default)]
struct ModuleSettlements {
    next: u64,
    jobs: BTreeMap<u64, ModuleSettlement>,
}
struct ModuleSettlement {
    environment: EnvironmentDescriptor,
    epoch: u64,
    source: v8::Global<v8::Promise>,
    resolver: v8::Global<v8::PromiseResolver>,
    namespace: v8::Global<v8::Value>,
    success: Option<v8::Global<v8::Function>>,
    failure: Option<v8::Global<v8::Function>>,
    attached: bool,
    outcome: Option<(bool, v8::Global<v8::Value>)>,
    terminal: bool,
    retry: Instant,
    deadline: Option<Instant>,
}
fn module_settlements(scope: &mut v8::PinScope) -> Rc<RefCell<ModuleSettlements>> {
    if let Some(jobs) = scope.get_slot::<Rc<RefCell<ModuleSettlements>>>() {
        return jobs.clone();
    }
    let jobs = Rc::new(RefCell::new(ModuleSettlements::default()));
    scope.set_slot(jobs.clone());
    jobs
}
fn retain_module_settlement(
    scope: &mut v8::PinScope,
    source: v8::Local<v8::Promise>,
    resolver: v8::Local<v8::PromiseResolver>,
    namespace: v8::Local<v8::Value>,
) -> Option<u64> {
    let environment = current_environment(scope).ok()?;
    let epoch = scope
        .get_current_context()
        .get_slot::<V8BindingState>()
        .map_or(0, |state| state.document_authority_epoch());
    let jobs = module_settlements(scope);
    let id = {
        let mut jobs = jobs.borrow_mut();
        if jobs.jobs.len() >= 1024 {
            return None;
        }
        let id = jobs.next.checked_add(1).filter(|id| *id <= u64::MAX / 2)?;
        jobs.next = id;
        id
    };
    let job = ModuleSettlement {
        environment,
        epoch,
        source: v8::Global::new(scope, source),
        resolver: v8::Global::new(scope, resolver),
        namespace: v8::Global::new(scope, namespace),
        success: None,
        failure: None,
        attached: false,
        outcome: None,
        terminal: false,
        retry: Instant::now(),
        deadline: Some(Instant::now() + Duration::from_secs(10)),
    };
    source.mark_as_handled();
    jobs.borrow_mut().jobs.insert(id, job);
    poll_module_settlement(scope, id);
    Some(id)
}
fn retain_module_rejection(
    scope: &mut v8::PinScope,
    resolver: v8::Local<v8::PromiseResolver>,
    error: v8::Local<v8::Value>,
) -> Option<u64> {
    let environment = current_environment(scope).ok()?;
    let epoch = scope
        .get_current_context()
        .get_slot::<V8BindingState>()
        .map_or(0, |state| state.document_authority_epoch());
    let jobs = module_settlements(scope);
    let id = {
        let mut jobs = jobs.borrow_mut();
        if jobs.jobs.len() >= 1024 {
            return None;
        }
        let id = jobs.next.checked_add(1).filter(|id| *id <= u64::MAX / 2)?;
        jobs.next = id;
        id
    };
    let promise = resolver.get_promise(scope);
    let error = v8::Global::new(scope, error);
    jobs.borrow_mut().jobs.insert(
        id,
        ModuleSettlement {
            environment,
            epoch,
            source: v8::Global::new(scope, promise),
            resolver: v8::Global::new(scope, resolver),
            namespace: error.clone(),
            success: None,
            failure: None,
            attached: true,
            outcome: Some((false, error)),
            terminal: false,
            retry: Instant::now(),
            deadline: Some(Instant::now() + Duration::from_secs(10)),
        },
    );
    poll_module_settlement(scope, id);
    Some(id)
}
fn module_settlement_callback(
    scope: &mut v8::PinScope,
    args: v8::FunctionCallbackArguments,
    mut result: v8::ReturnValue,
) {
    let Ok(data) = v8::Local::<v8::BigInt>::try_from(args.data()) else {
        return;
    };
    let (token, exact) = data.u64_value();
    if !exact {
        return;
    }
    let id = token >> 1;
    let success = token & 1 != 0;
    let jobs = module_settlements(scope);
    let Some(mut job) = jobs.borrow_mut().jobs.remove(&id) else {
        return;
    };
    if job.outcome.is_none() {
        let value = if success {
            job.namespace.clone()
        } else {
            v8::Global::new(scope, args.get(0))
        };
        job.outcome = Some((success, value));
        job.deadline = Some(Instant::now() + Duration::from_secs(10));
    }
    jobs.borrow_mut().jobs.insert(id, job);
    poll_module_settlement(scope, id);
    result.set_undefined();
}
fn poll_module_settlement(scope: &mut v8::PinScope, id: u64) {
    let jobs = module_settlements(scope);
    let Some(mut job) = jobs.borrow_mut().jobs.remove(&id) else {
        return;
    };
    if current_environment(scope).ok() != Some(job.environment) {
        jobs.borrow_mut().jobs.insert(id, job);
        return;
    }
    if let Some(state) = scope.get_current_context().get_slot::<V8BindingState>() {
        if state.execution_retired() || state.document_authority_epoch() != job.epoch {
            return;
        }
        if state.execution_suspended() {
            jobs.borrow_mut().jobs.insert(id, job);
            return;
        }
    }
    if job.terminal || scope.is_execution_terminating() {
        jobs.borrow_mut().jobs.insert(id, job);
        return;
    }
    v8::tc_scope!(let scope,scope);
    let complete = if let Some((success, value)) = &job.outcome {
        #[cfg(test)]
        if FAIL_MODULE_SETTLEMENT.with(|fail| fail.replace(false)) {
            job.retry = Instant::now() + Duration::from_millis(10);
            jobs.borrow_mut().jobs.insert(id, job);
            return;
        }
        let resolver = v8::Local::new(scope, &job.resolver);
        let value = v8::Local::new(scope, value);
        // V8 latches native resolver invocation before Get(then). Empty after
        // entry cannot be retried: the latch would return true for a stranded Promise.
        let accepted = if *success {
            resolver.resolve(scope, value) == Some(true)
        } else {
            resolver.reject(scope, value) == Some(true)
        };
        if !accepted {
            job.terminal = true;
        }
        accepted
    } else {
        if !job.attached {
            if job.success.is_none() {
                let token = v8::BigInt::new_from_u64(scope, (id << 1) | 1);
                job.success = v8::Function::builder(module_settlement_callback)
                    .data(token.into())
                    .build(scope)
                    .map(|function| v8::Global::new(scope, function));
            }
            if job.failure.is_none() {
                let token = v8::BigInt::new_from_u64(scope, id << 1);
                job.failure = v8::Function::builder(module_settlement_callback)
                    .data(token.into())
                    .build(scope)
                    .map(|function| v8::Global::new(scope, function));
            }
            if let (Some(success), Some(failure)) = (&job.success, &job.failure) {
                let source = v8::Local::new(scope, &job.source);
                let success = v8::Local::new(scope, success);
                let failure = v8::Local::new(scope, failure);
                job.attached = source.then2(scope, success, failure).is_some();
                if job.attached {
                    job.deadline = None;
                }
            }
        }
        false
    };
    if !complete {
        job.retry = Instant::now() + Duration::from_millis(10);
        jobs.borrow_mut().jobs.insert(id, job);
    }
    // Native resolver failure must not throw into the unrelated observer Promise.
    // Termination is reset by the owning host execution boundary.
    if scope.has_caught() {
        scope.reset();
    }
}
impl RawV8Host {
    fn module_settlement_delay(&self) -> Option<Duration> {
        let jobs = self.isolate.get_slot::<Rc<RefCell<ModuleSettlements>>>()?;
        let now = Instant::now();
        jobs.borrow()
            .jobs
            .values()
            .filter_map(|job| {
                if !self.direct_module_environment_live(&job.environment, job.epoch) {
                    return Some(Duration::ZERO);
                }
                if self.realm_is_parked(job.environment.realm) {
                    return job
                        .deadline
                        .map(|deadline| deadline.saturating_duration_since(now));
                }
                if job.terminal {
                    return Some(Duration::ZERO);
                }
                if job.attached && job.outcome.is_none() {
                    None
                } else {
                    Some(
                        job.retry
                            .min(job.deadline.unwrap_or(job.retry))
                            .saturating_duration_since(now),
                    )
                }
            })
            .min()
    }
    fn poll_module_settlements(&mut self) -> Result<usize, HostError> {
        let Some(jobs) = self
            .isolate
            .get_slot::<Rc<RefCell<ModuleSettlements>>>()
            .cloned()
        else {
            return Ok(0);
        };
        let now = Instant::now();
        let ids = jobs
            .borrow()
            .jobs
            .iter()
            .filter(|(_, job)| {
                !self.direct_module_environment_live(&job.environment, job.epoch)
                    || job.terminal
                    || job.deadline.is_some_and(|deadline| now >= deadline)
                    || (!self.realm_is_parked(job.environment.realm)
                        && (!job.attached || job.outcome.is_some())
                        && job.retry <= now)
            })
            .map(|(id, _)| *id)
            .take(64)
            .collect::<Vec<_>>();
        let previous = self.active_realm;
        let mut completed = 0;
        for id in ids {
            let Some((environment, epoch, retry, deadline, terminal)) =
                jobs.borrow().jobs.get(&id).map(|job| {
                    (
                        job.environment,
                        job.epoch,
                        job.retry,
                        job.deadline,
                        job.terminal,
                    )
                })
            else {
                continue;
            };
            if !self.direct_module_environment_live(&environment, epoch) {
                jobs.borrow_mut().jobs.remove(&id);
                continue;
            }
            if terminal {
                jobs.borrow_mut().jobs.remove(&id);
                if self.contexts.contains_key(&previous) {
                    self.active_realm = previous
                }
                return Err(HostError::Execution("native import resolver interrupted after invocation; settlement cannot be replayed".into()));
            }
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                jobs.borrow_mut().jobs.remove(&id);
                if self.contexts.contains_key(&previous) {
                    self.active_realm = previous
                }
                return Err(HostError::Execution(
                    "native import Promise settlement deadline exceeded".into(),
                ));
            }
            if retry > Instant::now() || self.realm_is_parked(environment.realm) {
                continue;
            }
            let Some(context) = self.contexts.get(&environment.realm).cloned() else {
                jobs.borrow_mut().jobs.remove(&id);
                continue;
            };
            self.active_realm = environment.realm;
            let deadline = ExecutionDeadline::arm(&self.isolate, self.execution_wall_limit);
            v8::scope!(let scope,&mut self.isolate);
            let context = v8::Local::new(scope, context);
            let scope = &mut v8::ContextScope::new(scope, context);
            poll_module_settlement(scope, id);
            if !jobs.borrow().jobs.contains_key(&id) {
                completed += 1;
                scope.perform_microtask_checkpoint();
            }
            if deadline.expired() || scope.is_execution_terminating() {
                scope.cancel_terminate_execution();
                reset_mutation_observer_microtask_after_termination(scope);
                if self.contexts.contains_key(&previous) {
                    self.active_realm = previous
                }
                return Err(HostError::LimitExceeded {
                    resource: "import settlement wall time".into(),
                });
            }
        }
        if self.contexts.contains_key(&previous) {
            self.active_realm = previous
        }
        Ok(completed)
    }
}
#[cfg(test)]
thread_local! {static FAIL_MODULE_SETTLEMENT:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}

#[cfg(test)]
mod module_settlement_tests {
    use super::*;
    #[test]
    fn exported_then_assimilates_once_into_the_original_import_promise() {
        let mut host = host();
        host.set_module_sources(&BTreeMap::from([("https://example.test/then.js".into(),
   "globalThis.evaluations++;export const marker=42;export function then(resolve,reject){globalThis.assimilations++;globalThis.receiverIsNamespace=this.marker===42;resolve(globalThis.answer);reject('ignored');resolve('ignored')}".into())]));
        host.execute("globalThis.evaluations=0;globalThis.assimilations=0;globalThis.receiverIsNamespace=false;globalThis.answer={value:17};globalThis.result='pending';import('./then.js').then(value=>result=value===answer?'same':'wrong',error=>result='rejected')").unwrap();
        let jobs = interrupted(&mut host);
        assert_eq!(
            host.evaluate_string("evaluations+'|'+assimilations+'|'+result")
                .unwrap(),
            "1|0|pending"
        );
        for job in jobs.borrow_mut().jobs.values_mut() {
            job.retry = Instant::now();
        }
        host.poll_module_settlements().unwrap();
        assert_eq!(
            host.evaluate_string(
                "evaluations+'|'+assimilations+'|'+receiverIsNamespace+'|'+result"
            )
            .unwrap(),
            "1|1|true|same"
        );
        host.poll_module_settlements().unwrap();
        assert_eq!(host.evaluate_string("String(assimilations)").unwrap(), "1");
    }
    fn host() -> RawV8Host {
        let mut host = RawV8Host::new_with_base_url_and_heap_limit(
            Url::parse("https://example.test/").unwrap(),
            256 * 1024 * 1024,
        );
        dynamic_import_continuation_tests::install_document(&mut host);
        host
    }
    fn interrupted(host: &mut RawV8Host) -> Rc<RefCell<ModuleSettlements>> {
        FAIL_MODULE_SETTLEMENT.with(|fail| fail.set(true));
        let limit = Instant::now() + Duration::from_secs(5);
        loop {
            host.drain_async_fetches(false).unwrap();
            if let Some(jobs) = host
                .isolate
                .get_slot::<Rc<RefCell<ModuleSettlements>>>()
                .cloned()
                && jobs.borrow().jobs.values().any(|job| job.outcome.is_some())
            {
                return jobs;
            }
            assert!(Instant::now() < limit, "settlement receipt not reached");
            std::thread::yield_now();
        }
    }
    #[test]
    fn import_callback_failure_retains_namespace_and_single_evaluation() {
        let mut host = host();
        host.set_module_sources(&BTreeMap::from([(
            "https://example.test/value.js".into(),
            "globalThis.runs=(globalThis.runs||0)+1;export const value=42;".into(),
        )]));
        host.execute(
            "globalThis.done=false;import('./value.js').then(ns=>{globalThis.result=ns;done=true})",
        )
        .unwrap();
        let jobs = interrupted(&mut host);
        assert_eq!(host.evaluate_string("runs+'|'+done").unwrap(), "1|false");
        assert!(host.module_settlement_delay().is_some());
        for job in jobs.borrow_mut().jobs.values_mut() {
            job.retry = Instant::now();
        }
        host.poll_module_settlements().unwrap();
        assert!(jobs.borrow().jobs.is_empty());
        assert_eq!(
            host.evaluate_string("runs+'|'+done+'|'+result.value")
                .unwrap(),
            "1|true|42"
        );
        host.poll_module_settlements().unwrap();
        assert_eq!(host.evaluate_string("String(runs)").unwrap(), "1");
    }
    #[test]
    fn rejection_keeps_original_author_exception_across_callback_failure() {
        let mut host = host();
        host.set_module_sources(&BTreeMap::from([(
            "https://example.test/bad.js".into(),
            "globalThis.runs=(globalThis.runs||0)+1;throw globalThis.sentinel;".into(),
        )]));
        host.execute("globalThis.sentinel={value:42};globalThis.done=false;import('./bad.js').catch(e=>{globalThis.same=e===sentinel;done=true})").unwrap();
        let jobs = interrupted(&mut host);
        assert_eq!(host.evaluate_string("String(done)").unwrap(), "false");
        for job in jobs.borrow_mut().jobs.values_mut() {
            job.retry = Instant::now();
        }
        host.poll_module_settlements().unwrap();
        assert_eq!(
            host.evaluate_string("same+'|'+done+'|'+runs").unwrap(),
            "true|true|1"
        );
    }
    #[test]
    fn waiting_tla_does_not_starve_ready_settlement_or_poll_at_zero_delay() {
        let mut host = host();
        let mut sources = BTreeMap::new();
        for index in 0..70 {
            sources.insert(
                format!("https://example.test/wait{index}.js"),
                "await new Promise(()=>{});export const value=0".into(),
            );
        }
        sources.insert(
            "https://example.test/ready.js".into(),
            "export const value=42".into(),
        );
        host.set_module_sources(&sources);
        host.execute("for(let i=0;i<70;i++) import('./wait'+i+'.js');")
            .unwrap();
        dynamic_import_continuation_tests::settle(&mut host);
        assert!(host.module_settlement_delay().is_none());
        host.execute("globalThis.done=false;import('./ready.js').then(ns=>{globalThis.value=ns.value;done=true})").unwrap();
        let jobs = interrupted(&mut host);
        assert!(jobs.borrow().jobs.len() > 64);
        for job in jobs.borrow_mut().jobs.values_mut() {
            job.retry = Instant::now();
        }
        host.poll_module_settlements().unwrap();
        assert_eq!(host.evaluate_string("done+'|'+value").unwrap(), "true|42");
        assert!(host.module_settlement_delay().is_none());
    }
    #[test]
    fn expired_settlement_releases_once_and_reports_native_failure() {
        let mut host = host();
        host.set_module_sources(&BTreeMap::from([(
            "https://example.test/value.js".into(),
            "export const value=42".into(),
        )]));
        host.execute("globalThis.done=false;import('./value.js').then(()=>done=true)")
            .unwrap();
        let jobs = interrupted(&mut host);
        for job in jobs.borrow_mut().jobs.values_mut() {
            job.deadline = Some(Instant::now());
        }
        assert!(host.poll_module_settlements().is_err());
        assert!(jobs.borrow().jobs.is_empty());
        assert_eq!(host.poll_module_settlements().unwrap(), 0);
        assert_eq!(host.evaluate_string("String(done)").unwrap(), "false");
    }
    #[test]
    fn parked_settlement_waits_for_restore_and_retirement_releases_roots() {
        let mut host = host();
        host.set_module_sources(&BTreeMap::from([(
            "https://example.test/value.js".into(),
            "export const value=42".into(),
        )]));
        host.execute("globalThis.done=false;import('./value.js').then(()=>done=true)")
            .unwrap();
        let jobs = interrupted(&mut host);
        let environment = jobs.borrow().jobs.values().next().unwrap().environment;
        host.isolate
            .get_slot_mut::<NativeRealmIdentities>()
            .unwrap()
            .parked_environments
            .insert(environment.id);
        for job in jobs.borrow_mut().jobs.values_mut() {
            job.retry = Instant::now();
        }
        assert!(host.module_settlement_delay().unwrap() > Duration::from_secs(1));
        assert_eq!(host.poll_module_settlements().unwrap(), 0);
        assert_eq!(jobs.borrow().jobs.len(), 1);
        host.isolate
            .get_slot_mut::<NativeRealmIdentities>()
            .unwrap()
            .parked_environments
            .remove(&environment.id);
        retire_environment(&mut host.isolate, environment.id);
        assert!(jobs.borrow().jobs.is_empty());
        assert!(host.module_settlement_delay().is_none());
        assert_eq!(host.poll_module_settlements().unwrap(), 0);
    }

    #[test]
    fn entered_then_getter_termination_is_terminal_not_a_successful_retry() {
        let mut host = host();
        host.execute("globalThis.gets=0;globalThis.ns={get then(){gets++;while(true){}}}")
            .unwrap();
        let queue = {
            let deadline = ExecutionDeadline::arm(&host.isolate, Duration::from_millis(20));
            v8::scope!(let handles,&mut host.isolate);
            let context = v8::Local::new(handles, &host.contexts[&host.active_realm]);
            let scope = &mut v8::ContextScope::new(handles, context);
            let key = v8::String::new(scope, "ns").unwrap();
            let ns = context.global(scope).get(scope, key.into()).unwrap();
            let source = v8::PromiseResolver::new(scope).unwrap();
            let undefined = v8::undefined(scope);
            assert_eq!(source.resolve(scope, undefined.into()), Some(true));
            let resolver = v8::PromiseResolver::new(scope).unwrap();
            let promise = source.get_promise(scope);
            retain_module_settlement(scope, promise, resolver, ns).unwrap();
            scope.perform_microtask_checkpoint();
            assert!(deadline.expired() || scope.is_execution_terminating());
            scope.cancel_terminate_execution();
            reset_mutation_observer_microtask_after_termination(scope);
            module_settlements(scope)
        };
        assert!(queue.borrow().jobs.values().all(|job| job.terminal));
        assert!(host.poll_module_settlements().is_err());
        assert!(queue.borrow().jobs.is_empty());
        assert_eq!(host.poll_module_settlements().unwrap(), 0);
        assert_eq!(host.evaluate_string("String(gets)").unwrap(), "1");
    }
    #[test]
    fn deferred_then_export_does_not_trigger_module_evaluation() {
        // Pinned V8 JSDeferredModuleNamespace::TriggersEvaluation excludes `then`
        // (module.cc); promise assimilation reads its uninitialized export without
        // running the deferred body, and therefore rejects with a TDZ error.
        let mut host = host();
        host.set_module_sources(&BTreeMap::from([(
            "https://example.test/deferred.js".into(),
            "globalThis.runs++;while(true){};export const then=undefined;".into(),
        )]));
        host.execute("globalThis.runs=0;globalThis.done=false;import.defer('./deferred.js').then(()=>done=true,e=>globalThis.deferredError=e)").unwrap();
        dynamic_import_continuation_tests::settle(&mut host);
        assert_eq!(
            host.evaluate_string("runs+'|'+done+'|'+(deferredError instanceof ReferenceError)")
                .unwrap(),
            "0|false|true"
        );
    }
    #[test]
    fn deferred_value_access_termination_never_replays_module_body() {
        let mut host = host();
        host.set_module_sources(&BTreeMap::from([(
            "https://example.test/deferred.js".into(),
            "globalThis.runs++;while(true){};export const value=42;".into(),
        )]));
        host.execute("globalThis.runs=0;import.defer('./deferred.js').then(ns=>globalThis.deferredNamespace=ns)").unwrap();
        dynamic_import_continuation_tests::settle(&mut host);
        assert_eq!(host.evaluate_string("String(runs)").unwrap(), "0");
        host.execution_wall_limit = Duration::from_millis(20);
        assert!(host.evaluate_string("deferredNamespace.value").is_err());
        assert_eq!(host.evaluate_string("String(runs)").unwrap(), "1");
        assert!(host.evaluate_string("deferredNamespace.value").is_err());
        assert_eq!(
            host.evaluate_string("String(runs)").unwrap(),
            "1",
            "terminated module body must never replay"
        );
    }

    #[test]
    fn native_typed_preparation_failure_retries_without_authored_type_error() {
        let mut host = host();
        host.retain_typed_module_fetch(
            "https://example.test/value.json",
            crate::ModuleType::Json,
            Some((
                "https://example.test/value.json".into(),
                "{\"value\":42}".into(),
            )),
        )
        .unwrap();
        host.execute("globalThis.result='pending';import('./value.json',{with:{type:'json'}}).then(ns=>result=ns.default.value,e=>result=e.name)").unwrap();
        FAIL_TYPED_PREPARATION.with(|failure| {
            *failure.borrow_mut() = Some(
                crate::script_load_failure::ScriptLoadFailure::Infrastructure(
                    "injected preparation failure".into(),
                ),
            )
        });
        let limit = Instant::now() + Duration::from_secs(5);
        loop {
            host.drain_async_fetches(false).unwrap();
            if host
                .isolate
                .get_slot::<Rc<RefCell<DynamicImports>>>()
                .unwrap()
                .borrow()
                .pending
                .front()
                .is_some_and(|pending| pending.native_failure.is_some())
            {
                break;
            }
            assert!(Instant::now() < limit);
            std::thread::yield_now();
        }
        assert_eq!(host.evaluate_string("String(result)").unwrap(), "pending");
        assert!(host.has_pending_dynamic_imports(false));
        let imports = host
            .isolate
            .get_slot::<Rc<RefCell<DynamicImports>>>()
            .unwrap()
            .clone();
        imports.borrow_mut().pending.front_mut().unwrap().retry = None;
        dynamic_import_continuation_tests::settle(&mut host);
        assert_eq!(host.evaluate_string("String(result)").unwrap(), "42");
    }
    #[test]
    fn native_typed_capacity_is_terminal_without_poisoning_module_map() {
        let mut host = host();
        host.retain_typed_module_fetch(
            "https://example.test/value.json",
            crate::ModuleType::Json,
            Some((
                "https://example.test/value.json".into(),
                "{\"value\":42}".into(),
            )),
        )
        .unwrap();
        host.execute("globalThis.result='pending';import('./value.json',{with:{type:'json'}}).then(ns=>result=ns.default.value,e=>result=e.name)").unwrap();
        FAIL_TYPED_PREPARATION.with(|failure| {
            *failure.borrow_mut() = Some(crate::script_load_failure::ScriptLoadFailure::Capacity(
                "injected capacity".into(),
            ))
        });
        dynamic_import_continuation_tests::settle(&mut host);
        assert!(!host.has_pending_dynamic_imports(false));
        assert_eq!(
            host.evaluate_string("String(result)").unwrap(),
            "TypeError",
            "live capacity failure must settle the original promise"
        );
        host.execute(
            "import('./value.json',{with:{type:'json'}}).then(ns=>result=ns.default.value)",
        )
        .unwrap();
        dynamic_import_continuation_tests::settle(&mut host);
        assert_eq!(host.evaluate_string("String(result)").unwrap(), "42");
    }
}
