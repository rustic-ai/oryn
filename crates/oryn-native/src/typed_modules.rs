// Native failure provenance is private to the active preparation call. It is
// never represented by an authored exception or persisted in the module map.
struct NativeModulePreparationFailure(crate::script_load_failure::ScriptLoadFailure);
fn record_module_preparation_failure(scope:&mut v8::PinScope,failure:crate::script_load_failure::ScriptLoadFailure){scope.get_current_context().set_slot(Rc::new(NativeModulePreparationFailure(failure)));}
fn module_preparation_empty(scope:&mut v8::PinScope,detail:&str)->String{if !scope.is_execution_terminating()&&scope.get_current_context().get_slot::<NativeModulePreparationFailure>().is_none(){record_module_preparation_failure(scope,crate::script_load_failure::ScriptLoadFailure::Infrastructure(detail.into()));}detail.into()}
fn take_module_preparation_failure(scope:&mut v8::PinScope)->Option<crate::script_load_failure::ScriptLoadFailure>{let context=scope.get_current_context();let failure=context.get_slot::<NativeModulePreparationFailure>().map(|entry|entry.0.clone());context.remove_slot::<NativeModulePreparationFailure>();failure}

// Native prefetched responses are keyed by URL AND type in the owning context.
// They contain no V8 handles and never merge JSON/CSS with JavaScript identity.
#[derive(Default)]
struct TypedModuleFetchResults(BTreeMap<(String, crate::ModuleType), Option<(String, String)>>, Rc<std::cell::Cell<u64>>, std::collections::BTreeSet<(String, crate::ModuleType)>);
impl TypedModuleFetchResults {
    fn retain(&mut self, requested: &str, kind: crate::ModuleType,
        result: Option<(&str, &str)>, _finish_pending: bool) -> Result<(), HostError> {
        if kind == crate::ModuleType::JavaScript {
            return Err(HostError::Execution("JavaScript entered typed prefetch cache".into()));
        }
        let key = (requested.to_owned(), kind);
        let existing = self.0.get(&key);
        if existing.is_some() {
            return Ok(());
        }
        let current = self.0.iter().fold(0usize, |bytes, ((url, _), value)| {
            bytes.saturating_add(url.len()).saturating_add(value.as_ref()
                .map_or(0, |(url, source)| url.len().saturating_add(source.len())))
        });
        let pending_bytes = self.2.iter().filter(|key| !self.0.contains_key(*key)).fold(0usize, |bytes, (url, _)| bytes.saturating_add(url.len()));
        let current = current.saturating_add(pending_bytes);
        let pending_count = self.2.iter().filter(|key| !self.0.contains_key(*key)).count();
        let added = if existing.is_some() || self.2.contains(&key) { 0 } else { requested.len() };
        let added = added.saturating_add(result.as_ref()
            .map_or(0, |(url, source)| url.len().saturating_add(source.len())));
        if (existing.is_none() && !self.2.contains(&key) && self.0.len() + pending_count >= 256) || current.saturating_add(added) > 32 * 1024 * 1024 {
            return Err(HostError::LimitExceeded { resource: "typed module response cache".into() });
        }
        self.0.insert(key, result.map(|(url, source)| (url.to_owned(), source.to_owned())));
        self.1.set(self.1.get().wrapping_add(1));
        Ok(())
    }
}
struct PendingTypedModuleFetch {
    cache: Rc<RefCell<TypedModuleFetchResults>>,
    key: (String, crate::ModuleType),
}
impl PendingTypedModuleFetch {
    fn reserve(cache: Rc<RefCell<TypedModuleFetchResults>>, requested: &str, kind: crate::ModuleType) -> Result<Self, HostError> {
        let key = (requested.to_owned(), kind);
        {
            let mut entries = cache.borrow_mut();
            if entries.0.contains_key(&key) || entries.2.contains(&key) {
                return Err(HostError::ScriptLoad(crate::script_load_failure::ScriptLoadFailure::Infrastructure("typed module request already owned".into())));
            }
            entries.retain(requested, kind, None, false)?;
            entries.0.remove(&key);
            entries.2.insert(key.clone());
        }
        Ok(Self { cache, key })
    }
}
impl Drop for PendingTypedModuleFetch {
    fn drop(&mut self) { self.cache.borrow_mut().2.remove(&self.key); }
}
fn typed_load_infrastructure(error: impl std::fmt::Display) -> crate::script_load_failure::ScriptLoadFailure {
    crate::script_load_failure::ScriptLoadFailure::Infrastructure(error.to_string())
}
fn typed_load_cache_failure(error: HostError) -> crate::script_load_failure::ScriptLoadFailure {
    match error {
        HostError::LimitExceeded { resource } => crate::script_load_failure::ScriptLoadFailure::Capacity(resource),
        HostError::ScriptLoad(failure) => failure,
        error => typed_load_infrastructure(error),
    }
}

fn typed_module_fetch_results(scope: &mut v8::PinScope) -> Rc<RefCell<TypedModuleFetchResults>> {
    register_module_context(scope);
    let context = scope.get_current_context();
    if let Some(results) = context.get_slot::<RefCell<TypedModuleFetchResults>>() { return results; }
    context.set_slot(Rc::new(RefCell::new(TypedModuleFetchResults::default())));
    context.get_slot::<RefCell<TypedModuleFetchResults>>().expect("typed module fetch cache installed")
}

#[cfg(test)]
mod typed_response_cache_tests {
    use super::*;
    #[test]
    fn pending_typed_requests_share_completed_cache_limits() {
        let cache = Rc::new(RefCell::new(TypedModuleFetchResults::default()));
        let mut owners = Vec::new();
        for id in 0..256 { owners.push(PendingTypedModuleFetch::reserve(cache.clone(), &format!("module:{id}"), crate::ModuleType::Json).unwrap()); }
        assert!(PendingTypedModuleFetch::reserve(cache.clone(), "overflow", crate::ModuleType::Css).is_err());
        cache.borrow_mut().retain("module:0", crate::ModuleType::Json, Some(("final", "{}")), false).unwrap();
        assert!(cache.borrow_mut().retain("other", crate::ModuleType::Css, None, false).is_err());
        owners.pop();
        let replacement = PendingTypedModuleFetch::reserve(cache.clone(), "replacement", crate::ModuleType::Css).unwrap();
        drop(owners); drop(replacement);
        assert_eq!(cache.borrow().0.len(), 1);
        assert!(cache.borrow().2.is_empty());
    }
    #[test]
    fn typed_pending_identity_is_type_scoped_and_does_not_poison_failures() {
        let cache = Rc::new(RefCell::new(TypedModuleFetchResults::default()));
        let json = PendingTypedModuleFetch::reserve(cache.clone(), "x", crate::ModuleType::Json).unwrap();
        let css = PendingTypedModuleFetch::reserve(cache.clone(), "x", crate::ModuleType::Css).unwrap();
        assert!(PendingTypedModuleFetch::reserve(cache.clone(), "x", crate::ModuleType::Json).is_err());
        assert!(cache.borrow().0.is_empty());
        drop(json);
        let json = PendingTypedModuleFetch::reserve(cache.clone(), "x", crate::ModuleType::Json).unwrap();
        cache.borrow_mut().retain("x", crate::ModuleType::Json, None, false).unwrap();
        cache.borrow_mut().retain("x", crate::ModuleType::Json, Some(("late", "{}")), true).unwrap();
        drop(json); drop(css);
        assert_eq!(cache.borrow().0.get(&("x".into(), crate::ModuleType::Json)), Some(&None));
        assert!(cache.borrow().2.is_empty());
    }
    #[test]
    fn failed_entries_consume_capacity_and_existing_results_remain_sticky() {
        let mut cache = TypedModuleFetchResults::default();
        for index in 0..256 {
            cache.retain(&format!("module:{index}"), crate::ModuleType::Json, None, false).unwrap();
        }
        assert!(matches!(cache.retain("overflow", crate::ModuleType::Json, None, false),
            Err(HostError::LimitExceeded { .. })));
        cache.retain("module:0", crate::ModuleType::Json,
            Some(("redirect", "{}")), false).unwrap();
        assert_eq!(cache.0.get(&("module:0".into(), crate::ModuleType::Json)), Some(&None));
        cache.retain("module:0", crate::ModuleType::Json,
            Some(("redirect", "{}")), true).unwrap();
        cache.retain("module:0", crate::ModuleType::Json,
            Some(("different", "null")), true).unwrap();
        assert_eq!(cache.0.get(&("module:0".into(), crate::ModuleType::Json)),
            Some(&None));
    }
    #[test]
    fn pending_completion_budget_counts_keys_redirect_and_body_before_publication() {
        let mut cache = TypedModuleFetchResults::default();
        cache.2.insert(("x".into(), crate::ModuleType::Json));
        let source = " ".repeat(32 * 1024 * 1024 - 1);
        assert!(matches!(cache.retain("x", crate::ModuleType::Json,
            Some(("y", &source)), true),
            Err(HostError::LimitExceeded { .. })));
        assert_eq!(cache.0.get(&("x".into(), crate::ModuleType::Json)), None);
        cache.retain("x", crate::ModuleType::Json,
            Some(("y", &source[..source.len()-1])), true).unwrap();
        assert!(cache.retain("x", crate::ModuleType::Css, None, false).is_err());
        assert_eq!(cache.0.len(), 1);
    }
}

// Included in raw_v8 so typed modules share native environment and fetch authority.
#[derive(Default)]
struct TypedModules {
    entries: BTreeMap<(EnvironmentId, String, crate::ModuleType), Option<Result<v8::Global<v8::Module>, v8::Global<v8::Value>>>>,
    values: Vec<(EnvironmentId, v8::Global<v8::Module>, v8::Global<v8::Value>)>,
    source_bytes: BTreeMap<EnvironmentId, usize>,
}

fn evaluate_typed_module<'s>(context: v8::Local<'s, v8::Context>, module: v8::Local<'s, v8::Module>) -> Option<v8::Local<'s, v8::Value>> {
    v8::callback_scope!(unsafe scope, context);
    let environment=current_environment(scope).ok()?;
    let value = scope.get_slot::<TypedModules>()?.values.iter().find(|(owner,record,_)| *owner==environment.id && record==&module)?.2.clone();
    let value = v8::Local::new(scope, value);
    let name = v8::String::new(scope, "default")?;
    module.set_synthetic_module_export(scope, name, value)?;
    let resolver = v8::PromiseResolver::new(scope)?;
    let undefined = v8::undefined(scope);
    resolver.resolve(scope, undefined.into())?;
    Some(resolver.get_promise(scope).into())
}

fn typed_module_source(scope: &mut v8::PinScope, base: &Url, url: &Url,
    kind: crate::ModuleType, include: bool, policy: &str) -> Result<(String, String), crate::script_load_failure::ScriptLoadFailure> {
    use crate::script_load_failure::ScriptLoadFailure;
    let environment = current_environment(scope).map_err(typed_load_infrastructure)?;
    let results = typed_module_fetch_results(scope);
    let key = (url.to_string(), kind);
    let failed = || ScriptLoadFailure::Fetch("typed module fetch previously failed".into());
    if let Some(result) = results.borrow().0.get(&key).cloned() { return result.ok_or_else(failed); }
    if scope.get_current_context().get_slot::<PreparedImportLink>().is_some(){return Err(typed_load_infrastructure("prepared typed module response missing"));}
    let _=(environment,base,include,policy);
    Err(typed_load_infrastructure("typed module source was not prepared"))
}

fn load_typed_module_record<'s>(scope: &mut v8::PinScope<'s, '_>, base: &str, specifier: &str,
    kind: crate::ModuleType, include: bool, policy: &str) -> Option<v8::Local<'s, v8::Module>> {
    let environment = current_environment(scope).ok()?;
    let base = Url::parse(base).ok().or_else(|| {
        let state = scope.get_current_context().get_slot::<V8BindingState>()?;
        let base = oryn_js::v8_bindings::document_base_url(scope, &state, true).ok()?;
        Url::parse(&base).ok()
    })?;
    let map = module_import_map(scope);
    let captured=prepared_import_root(scope);
    let captured_root=captured.is_some();
    let resolution=match captured{Some(root)=>Ok(Some(root)),None=>map.borrow().resolve(&base,specifier)};
    let resolved = match resolution {
        Ok(Some(url)) => url,
        Ok(None) => {
            let message = v8::String::new(scope, "unmapped typed module specifier")?;
            let exception = v8::Exception::type_error(scope, message); scope.throw_exception(exception); return None;
        }
        Err(error) => {
            let message = v8::String::new(scope, &error)?;
            let exception = v8::Exception::type_error(scope, message); scope.throw_exception(exception); return None;
        }
    };
    if !captured_root{
        let recorded=map.borrow_mut().record_resolution(&base,specifier);
        if let Err(error)=recorded{record_module_preparation_failure(scope,crate::script_load_failure::ScriptLoadFailure::Capacity(error));return None;}
    }
    let key = (environment.id, resolved.clone(), kind);
    let cached = scope.get_slot::<TypedModules>()?.entries.get(&key).cloned();
    if let Some(cached) = cached {
        return match cached {
            Some(Ok(module)) => {
                let module=v8::Local::new(scope,module);
                if kind==crate::ModuleType::Css{if let Err(error)=publish_declarative_css_module(scope,&resolved,Some(module)){
                    record_module_preparation_failure(scope,typed_load_infrastructure(error));return None;
                }}
                Some(module)
            },
            Some(Err(exception)) => { let exception = v8::Local::new(scope, exception); scope.throw_exception(exception); None }
            None => { let message = v8::String::new(scope, "typed module fetch is already in progress")?;
                let exception = v8::Exception::type_error(scope, message); scope.throw_exception(exception); None }
        };
    }
    if scope.get_slot::<TypedModules>()?.entries.keys().filter(|(id, _, _)| *id == environment.id).count() >= 256 {
        record_module_preparation_failure(scope,crate::script_load_failure::ScriptLoadFailure::Capacity("typed module graph entries".into()));return None;
    }
    scope.get_slot_mut::<TypedModules>()?.entries.insert(key.clone(), None);
    #[cfg(test)]if let Some(failure)=FAIL_TYPED_PREPARATION.with(|failure|failure.borrow_mut().take()){scope.get_slot_mut::<TypedModules>()?.entries.remove(&key);record_module_preparation_failure(scope,failure);return None;}

    v8::tc_scope!(let scope, scope);
    let mut cache_failure = false;
    let mut native_failure=None;
    let mut resource_failure=false;
    let mut parse_url=None;
    let result = (|| -> Result<v8::Local<'s, v8::Module>, String> {
        let url = Url::parse(&resolved).map_err(|error| error.to_string())?;
        let (final_url, source) = typed_module_source(scope, &base, &url, kind, include, policy).map_err(|failure| {
            cache_failure = failure.is_fetch_failure();
            resource_failure=matches!(failure,crate::script_load_failure::ScriptLoadFailure::Fetch(_)|crate::script_load_failure::ScriptLoadFailure::Integrity(_));
            if matches!(failure,crate::script_load_failure::ScriptLoadFailure::Infrastructure(_)|crate::script_load_failure::ScriptLoadFailure::Capacity(_)|crate::script_load_failure::ScriptLoadFailure::Cancelled){native_failure=Some(failure.clone());}
            failure.to_string()
        })?;
        if current_environment(scope)?.id != environment.id { return Err("typed module environment retired".into()); }
        let used = scope.get_slot_mut::<TypedModules>().ok_or("typed module registry unavailable")?
            .source_bytes.entry(environment.id).or_default();
        let next = used.checked_add(source.len()).filter(|size| *size <= 16 * 1024 * 1024)
            .ok_or_else(||{native_failure=Some(crate::script_load_failure::ScriptLoadFailure::Capacity("typed module source bytes".into()));"typed module graph source limit exceeded"})?;
        scope.get_slot_mut::<TypedModules>().ok_or("typed module registry unavailable")?.values.try_reserve(1).map_err(|_|"typed module value custody allocation")?;
        let value = match kind {
            crate::ModuleType::Json => {
                let source = v8::String::new(scope, &source).ok_or("JSON module allocation failed")?;
                #[cfg(test)] let parsed=if FAIL_JSON_PARSE_EMPTY.with(|failure|failure.replace(false)){None}else{v8::json::parse(scope,source)};
                #[cfg(not(test))] let parsed=v8::json::parse(scope,source);
                match parsed{
                    Some(value)=>value,
                    None=>{
                        if scope.exception().is_some(){cache_failure=true;parse_url=Some(final_url.clone());}
                        else{native_failure=Some(crate::script_load_failure::ScriptLoadFailure::Infrastructure("JSON parser returned without a JavaScript exception".into()));}
                        return Err("JSON module parsing failed".into());
                    }
                }
            }
            crate::ModuleType::Text => v8::String::new(scope, &source).ok_or("text module allocation failed")?.into(),
            crate::ModuleType::Css => oryn_js::v8_bindings::create_module_stylesheet(scope, &source, &final_url)
                .map_err(|error| error.to_string())?,
            crate::ModuleType::JavaScript => return Err("JavaScript entered the typed data module path".into()),
        };
        let name = v8::String::new(scope, &final_url).ok_or("typed module URL allocation failed")?;
        let export = v8::String::new(scope, "default").ok_or("typed module export allocation failed")?;
        let module = v8::Module::create_synthetic_module(scope, name, &[export], evaluate_typed_module);
        let value = v8::Global::new(scope, value);
        let record = v8::Global::new(scope,module);
        scope.get_slot_mut::<TypedModules>().ok_or("typed module registry unavailable")?
            .values.push((environment.id, record, value));
        scope.get_slot_mut::<TypedModules>().ok_or("typed module registry unavailable")?
            .source_bytes.insert(environment.id, next);
        retain_module_credentials(scope, module, &final_url);
        retain_module_name(scope,module,final_url);
        Ok(module)
    })();
    match result {
        Ok(module) => {
            let retained = v8::Global::new(scope, module);
            scope.get_slot_mut::<TypedModules>()?.entries.insert(key, Some(Ok(retained)));
            if kind==crate::ModuleType::Css{if let Err(error)=publish_declarative_css_module(scope,&resolved,Some(module)){
                record_module_preparation_failure(scope,typed_load_infrastructure(error));return None;
            }}
            Some(module)
        }
        Err(error) => {
            // Release local/admission failures before fallible exception allocation.
            if !cache_failure { scope.get_slot_mut::<TypedModules>()?.entries.remove(&key); }
            if scope.is_execution_terminating() {
                scope.get_slot_mut::<TypedModules>()?.entries.remove(&key);
                scope.rethrow(); return None;
            }
            if let Some(failure)=native_failure{record_module_preparation_failure(scope,failure);return None;}
            if !cache_failure&&!resource_failure&&scope.exception().is_none(){record_module_preparation_failure(scope,crate::script_load_failure::ScriptLoadFailure::Infrastructure(error));return None;}
            let exception = if let Some(exception) = scope.exception() { exception } else {
                let message = v8::String::new(scope, &error)?;
                let exception = v8::Exception::type_error(scope, message); scope.throw_exception(exception); exception
            };
            if let Some(url)=parse_url{if V8BindingState::set_native_exception_location(scope,exception,&url,0,0).is_err(){scope.get_slot_mut::<TypedModules>()?.entries.remove(&key);scope.rethrow();return None;}}
            let retained = v8::Global::new(scope, exception);
            if cache_failure {
                scope.get_slot_mut::<TypedModules>()?.entries.insert(key, Some(Err(retained)));
            } else {
                scope.get_slot_mut::<TypedModules>()?.entries.remove(&key);
            }
            scope.rethrow(); None
        }
    }
}

#[cfg(test)]thread_local!{static FAIL_JSON_PARSE_EMPTY:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}
#[cfg(test)]thread_local!{static FAIL_TYPED_PREPARATION:RefCell<Option<crate::script_load_failure::ScriptLoadFailure>>=const{RefCell::new(None)};}


#[cfg(test)]
mod typed_resolution_capacity_tests{
    use super::*;
    #[test]
    fn graph_json_empty_parse_preserves_native_failure_and_can_retry(){
        let url="https://example.test/value.json";
        let mut host=RawV8Host::new_with_base_url_and_heap_limit(Url::parse("https://example.test/").unwrap(),256*1024*1024);
        host.retain_typed_module_fetch(url,crate::ModuleType::Json,Some((url.into(),"42".into()))).unwrap();
        FAIL_JSON_PARSE_EMPTY.with(|failure|failure.set(true));
        assert!(matches!(host.prepare_json_graph_error(url),Err(HostError::ScriptLoad(crate::script_load_failure::ScriptLoadFailure::Infrastructure(_)))));
        assert!(host.prepare_json_graph_error(url).unwrap().is_none());
    }

    #[test]
    fn empty_json_parse_without_exception_is_retryable_not_sticky(){
        let url="https://example.test/value.json";
        let mut host=RawV8Host::new_with_base_url_and_heap_limit(Url::parse("https://example.test/").unwrap(),256*1024*1024);
        host.retain_typed_module_fetch(url,crate::ModuleType::Json,Some((url.into(),"{\"value\":42}".into()))).unwrap();
        let context=host.contexts.get(&host.active_realm).cloned().unwrap();
        v8::scope!(let scope,&mut host.isolate);let context=v8::Local::new(scope,&context);let scope=&mut v8::ContextScope::new(scope,context);
        FAIL_JSON_PARSE_EMPTY.with(|failure|failure.set(true));
        assert!(load_typed_module_record(scope,"https://example.test/",url,crate::ModuleType::Json,false,"no-referrer").is_none());
        assert!(matches!(take_module_preparation_failure(scope),Some(crate::script_load_failure::ScriptLoadFailure::Infrastructure(_))));
        assert!(load_typed_module_record(scope,"https://example.test/",url,crate::ModuleType::Json,false,"no-referrer").is_some());
    }

    #[test]
    fn resolution_budget_failure_keeps_native_capacity_provenance(){
        let mut host=RawV8Host::new_with_base_url_and_heap_limit(Url::parse("https://example.test/").unwrap(),256*1024*1024);
        let context=host.contexts.get(&host.active_realm).cloned().unwrap();
        v8::scope!(let scope,&mut host.isolate);let context=v8::Local::new(scope,&context);let scope=&mut v8::ContextScope::new(scope,context);
        let base=Url::parse("https://example.test/").unwrap();let map=module_import_map(scope);
        for index in 0..8192{map.borrow_mut().record_resolution(&base,&format!("./resolved-{index}.js")).unwrap();}
        assert!(load_typed_module_record(scope,base.as_str(),"./new.json",crate::ModuleType::Json,false,"no-referrer").is_none());
        assert!(matches!(take_module_preparation_failure(scope),Some(crate::script_load_failure::ScriptLoadFailure::Capacity(_))));
        assert!(take_module_preparation_failure(scope).is_none(),"native failure belongs to exactly one preparation");
    }
}
