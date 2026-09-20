// Native modulepreload admission, module-map preparation and retained events.
// Included in raw_v8; no module body executes on this path.
#[derive(Clone, PartialEq, Eq)]
struct ModulePreloadKey {
    url: String,
    kind: Option<crate::ModuleType>,
    include: bool,
    policy: String,
    integrity: Option<String>,
    base: String,
    media: String,
    nonce: Option<String>,
    parser_inserted: bool,
}
impl ModulePreloadKey {
    fn retained_bytes(&self) -> usize {
        self.url
            .len()
            .saturating_add(self.base.len())
            .saturating_add(self.policy.len())
            .saturating_add(self.media.len())
            .saturating_add(self.integrity.as_ref().map_or(0, String::len))
            .saturating_add(self.nonce.as_ref().map_or(0, String::len))
    }
}
#[derive(Default)]
struct ModulePreloads {
    jobs: std::collections::VecDeque<ModulePreload>,
    seen: BTreeMap<(EnvironmentId, NodeId), ModulePreloadKey>,
    headers: BTreeSet<(EnvironmentId, String)>,
}
struct ModulePreload {
    environment: EnvironmentDescriptor,
    owner: Rc<V8BindingState>,
    epoch: u64,
    cancellation: u64,
    key: ModulePreloadKey,
    target: Option<oryn_dom::TrackedNodeLease>,
    graph: Option<ModuleGraphLoad>,
    backend: ModuleGraphBackend,
    preparation_pauses: Rc<std::cell::Cell<usize>>,
    result: Option<bool>,
    publication: Option<u64>,
    deadline: Instant,
    publication_deadline: Option<Instant>,
    retry: Instant,
    first_error: Option<String>,
}
// A parser checkpoint retains baseline preparation without consuming jobs,
// extending their graph under a provisional import map, or publishing events.
struct ModulePreloadPause(Rc<std::cell::Cell<usize>>);
impl Drop for ModulePreloadPause {
    fn drop(&mut self) {
        self.0.set(self.0.get() - 1);
    }
}
impl ModulePreload {
    fn pause_preparation(&self) -> ModulePreloadPause {
        self.preparation_pauses.set(
            self.preparation_pauses
                .get()
                .checked_add(1)
                .expect("bounded parser checkpoint nesting"),
        );
        ModulePreloadPause(self.preparation_pauses.clone())
    }
}
impl ModulePreload {
    fn finish_fetch(&mut self, result: bool) {
        self.result = Some(result);
        self.publication_deadline = Some(Instant::now() + Duration::from_secs(10));
        self.graph.take();
    }
}
impl Drop for ModulePreload {
    fn drop(&mut self) {
        if let Some(receipt) = self.publication.take() {
            oryn_js::navigation_api::cancel_native_history_receipt(&self.owner, receipt);
        }
    }
}
fn module_preloads(scope: &mut v8::PinScope) -> Rc<RefCell<ModulePreloads>> {
    if let Some(queue) = scope.get_slot::<Rc<RefCell<ModulePreloads>>>() {
        return queue.clone();
    }
    let queue = Rc::new(RefCell::new(ModulePreloads::default()));
    scope.set_slot(queue.clone());
    queue
}
fn capture_module_preloads(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    root: NodeId,
) -> Result<(), oryn_js::v8_bindings::V8BindingError> {
    capture_module_preloads_inner(scope, state, root, false)
}
fn capture_module_preloads_inner(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    root: NodeId,
    parser_inserted: bool,
) -> Result<(), oryn_js::v8_bindings::V8BindingError> {
    oryn_js::v8_bindings::prepare_declarative_stylesheets(scope, state, root)?;
    capture_general_preloads(scope, state, root, parser_inserted)?;
    use oryn_js::v8_bindings::V8BindingError;
    if state.execution_retired() || state.document_load_stopped() {
        return Ok(());
    }
    let base = oryn_js::v8_bindings::document_base_url(scope, state, true)?;
    let environment = current_environment(scope).map_err(V8BindingError::EventCallback)?;
    let queue = module_preloads(scope);
    let dom = state.dom();
    let nodes = {
        let dom = dom.borrow();
        dom.arena().descendants_inclusive(root)?
    };
    for node in nodes {
        let attributes = {
            let dom = dom.borrow();
            if !dom.is_connected(node)?
                || !dom.is_element(node, "http://www.w3.org/1999/xhtml", "link")
            {
                continue;
            }
            let attr = |name| {
                dom.attribute_in_null_namespace(node, name)
                    .map(|value| value.unwrap_or("").to_owned())
            };
            if !attr("rel")?
                .split_ascii_whitespace()
                .any(|token| token.eq_ignore_ascii_case("modulepreload"))
            {
                queue.borrow_mut().seen.remove(&(environment.id, node));
                continue;
            }
            (
                attr("href")?,
                attr("as")?,
                attr("crossorigin")?,
                attr("referrerpolicy")?,
                attr("integrity")?,
                attr("media")?,
                dom.element_nonce_for_csp(node)?.map(str::to_owned),
            )
        };
        if attributes.0.is_empty() {
            queue.borrow_mut().seen.remove(&(environment.id, node));
            continue;
        }
        if !attributes.5.is_empty()
            && !state
                .native_media_matches(&attributes.5)
                .map_err(V8BindingError::EventCallback)?
        {
            queue.borrow_mut().seen.remove(&(environment.id, node));
            continue;
        }
        let resolved = state.native_document_resource_url(&attributes.0).ok();
        let url = resolved
            .as_ref()
            .map_or_else(|| attributes.0.clone(), Url::to_string);
        let kind = match attributes.1.to_ascii_lowercase().as_str() {
            "" | "script" | "worker" | "sharedworker" | "serviceworker" | "audioworklet"
            | "paintworklet" => Some(crate::ModuleType::JavaScript),
            "json" => Some(crate::ModuleType::Json),
            "style" => Some(crate::ModuleType::Css),
            "text" => Some(crate::ModuleType::Text),
            _ => None,
        };
        let kind = if resolved.is_some() { kind } else { None };
        let policy = crate::referrer::ReferrerPolicy::parse_attribute(&attributes.3)
            .map(|policy| policy.as_str().to_owned())
            .unwrap_or_else(|| state.document_referrer_policy());
        let integrity = if attributes.4.is_empty() {
            module_import_map(scope)
                .borrow()
                .integrity_for(&url)
                .map(str::to_owned)
        } else {
            Some(attributes.4)
        };
        let key = ModulePreloadKey {
            url: url.to_string(),
            kind,
            include: attributes.2.eq_ignore_ascii_case("use-credentials"),
            policy,
            integrity,
            base: base.clone(),
            media: attributes.5,
            nonce: attributes.6,
            parser_inserted,
        };
        if queue.borrow().seen.get(&(environment.id, node)) == Some(&key) {
            continue;
        }
        let target = oryn_dom::TrackedNodeLease::new(&dom, node)?;
        admit_module_preload(scope, state, Some(target), key.clone())
            .map_err(|error| V8BindingError::EventCallback(error.to_string()))?;
        queue.borrow_mut().seen.insert((environment.id, node), key);
    }
    Ok(())
}
fn admit_module_preload(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    target: Option<oryn_dom::TrackedNodeLease>,
    key: ModulePreloadKey,
) -> Result<(), HostError> {
    let queue = module_preloads(scope);
    let metadata = key.retained_bytes();
    let retained = {
        let queue = queue.borrow();
        queue
            .seen
            .values()
            .map(ModulePreloadKey::retained_bytes)
            .chain(queue.jobs.iter().map(|job| job.key.retained_bytes()))
            .chain(queue.headers.iter().map(|(_, value)| value.len()))
            .fold(0usize, usize::saturating_add)
    };
    if metadata > 192 * 1024
        || retained.saturating_add(metadata.saturating_mul(2)) > 32 * 1024 * 1024
    {
        return Err(HostError::LimitExceeded {
            resource: "modulepreload metadata bytes".into(),
        });
    }
    if queue.borrow().jobs.len() >= 256 || queue.borrow().seen.len() >= 4096 {
        return Err(HostError::LimitExceeded {
            resource: "modulepreload admission".into(),
        });
    }
    queue
        .borrow_mut()
        .jobs
        .try_reserve(1)
        .map_err(|_| HostError::LimitExceeded {
            resource: "modulepreload queue".into(),
        })?;
    let environment = current_environment(scope).map_err(HostError::Execution)?;
    let backend = module_fetch_backend_in_scope(scope)?;
    let context = scope.get_current_context();
    let limits = context.get_slot::<DynamicImportLimits>();
    let imports = scope
        .get_slot::<Rc<RefCell<DynamicImports>>>()
        .ok_or_else(|| HostError::Execution("module budget absent".into()))?
        .clone();
    let (bytes, slots, fetches) = limits
        .map(|limits| {
            (
                limits.bytes.clone(),
                limits.slots.clone(),
                limits.fetches.clone(),
            )
        })
        .unwrap_or_else(|| {
            let imports = imports.borrow();
            (
                imports.bytes.clone(),
                imports.slots.clone(),
                imports.fetches.clone(),
            )
        });
    let graph = key
        .kind
        .map(|kind| {
            ModuleGraphLoad::external_root(crate::executable::module_graph_load::ExternalModuleRootInput { requested: &key.url, parent: &key.base, kind, include: key.include, policy: &key.policy, integrity: key.integrity.clone(), budget: bytes, slots, registry: fetches })
            .and_then(|graph| {
                graph.with_root_request_metadata(key.nonce.clone(), key.parser_inserted)
            })
        })
        .transpose()?;
    let result = graph.is_none().then_some(false);
    queue.borrow_mut().jobs.push_back(ModulePreload {
        environment,
        owner: state.clone(),
        epoch: state.document_authority_epoch(),
        cancellation: state.document_load_cancellation_epoch(),
        key,
        target,
        graph,
        backend,
        preparation_pauses: Default::default(),
        result,
        publication: None,
        deadline: Instant::now() + Duration::from_secs(60),
        publication_deadline: result.map(|_| Instant::now() + Duration::from_secs(10)),
        retry: Instant::now(),
        first_error: None,
    });
    Ok(())
}
impl RawV8Host {
    pub(crate) fn prepare_module_preload(&mut self, node: NodeId) -> Result<(), HostError> {
        let state = self
            .binding_states
            .get(&self.active_realm)
            .cloned()
            .ok_or_else(|| HostError::Execution("preload realm absent".into()))?;
        let context = self
            .contexts
            .get(&self.active_realm)
            .cloned()
            .ok_or_else(|| HostError::Execution("preload context absent".into()))?;
        v8::scope!(let scope,&mut self.isolate);
        let context = v8::Local::new(scope, context);
        let scope = &mut v8::ContextScope::new(scope, context);
        capture_module_preloads_inner(scope, &state, node, true)
            .map_err(|error| HostError::Execution(error.to_string()))
    }
    fn module_preload_delay(&self) -> Option<Duration> {
        let queue = self.isolate.get_slot::<Rc<RefCell<ModulePreloads>>>()?;
        let now = Instant::now();
        queue
            .borrow()
            .jobs
            .iter()
            .filter_map(|job| {
                if job.owner.execution_retired()
                    || job.epoch != job.owner.document_authority_epoch()
                {
                    return Some(Duration::ZERO);
                }
                if job.preparation_pauses.get() != 0 {
                    return None;
                }
                if job.owner.execution_suspended() {
                    return job
                        .publication_deadline
                        .map(|deadline| deadline.saturating_duration_since(now));
                }
                Some(
                    if job.result.is_some()
                        || job.graph.as_ref().is_some_and(ModuleGraphLoad::is_runnable)
                    {
                        job.retry
                            .min(job.publication_deadline.unwrap_or(job.deadline))
                            .saturating_duration_since(now)
                    } else {
                        job.deadline
                            .saturating_duration_since(now)
                            .min(Duration::from_millis(10))
                    },
                )
            })
            .min()
    }
    fn poll_module_preloads(&mut self) -> Result<usize, HostError> {
        let Some(queue) = self
            .isolate
            .get_slot::<Rc<RefCell<ModulePreloads>>>()
            .cloned()
        else {
            return Ok(0);
        };
        let count = queue.borrow().jobs.len();
        let previous = self.active_realm;
        let mut complete = 0;
        for _ in 0..count.min(32) {
            let Some(mut job) = queue.borrow_mut().jobs.pop_front() else {
                break;
            };
            if job.owner.execution_retired()
                || job.epoch != job.owner.document_authority_epoch()
                || job.cancellation != job.owner.document_load_cancellation_epoch()
            {
                continue;
            }
            if job.preparation_pauses.get() != 0 {
                queue.borrow_mut().jobs.push_back(job);
                continue;
            }
            if job
                .publication_deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
            {
                let detail = job
                    .first_error
                    .take()
                    .unwrap_or_else(|| "publication remained pending".into());
                drop(job);
                if self.contexts.contains_key(&previous) {
                    self.active_realm = previous
                }
                return Err(HostError::Execution(format!(
                    "modulepreload publication deadline exceeded: {detail}"
                )));
            }
            if job.owner.execution_suspended() || job.retry > Instant::now() {
                queue.borrow_mut().jobs.push_back(job);
                continue;
            }
            if !self
                .isolate
                .get_slot::<AgentEnvironments>()
                .is_some_and(|environments| environments.is_live(&job.environment))
            {
                continue;
            }
            self.active_realm = job.environment.realm;
            let result = (|| -> Result<bool, HostError> {
                if job.result.is_none() {
                    if Instant::now() >= job.deadline {
                        job.finish_fetch(false)
                    } else {
                        match job
                            .graph
                            .as_mut()
                            .expect("preload graph")
                            .poll_with_backend(self, &job.backend, &mut BTreeMap::new())
                        {
                            Ok(false) => return Ok(false),
                            Ok(true) => {
                                self.cache_preloaded_module(&job.key)?;
                                job.finish_fetch(true)
                            }
                            Err(HostError::ModuleSyntax(_)) => job.finish_fetch(true),
                            Err(HostError::ScriptLoad(
                                crate::script_load_failure::ScriptLoadFailure::Fetch(_)
                                | crate::script_load_failure::ScriptLoadFailure::Integrity(_),
                            )) => {
                                // Descendant prefetch is optional. Only the requested root determines
                                // this link's load/error event; descendants retain their own map result.
                                let loaded = match job.key.kind {
                                    Some(crate::ModuleType::JavaScript) => {
                                        self.retained_module_fetch(&job.key.url).flatten().is_some()
                                    }
                                    Some(kind) => self
                                        .retained_typed_module_fetch(&job.key.url, kind)
                                        .flatten()
                                        .is_some(),
                                    None => false,
                                };
                                if loaded {
                                    self.cache_preloaded_module(&job.key)?;
                                }
                                job.finish_fetch(loaded);
                            }
                            Err(error) => return Err(error),
                        }
                    }
                }
                #[cfg(test)]
                if FAIL_PRELOAD_PUBLICATION.with(|fail| fail.replace(false)) {
                    return Err(HostError::Execution(
                        "injected preload publication failure".into(),
                    ));
                }
                self.publish_declarative_css_preload(&job.key, job.result.unwrap())?;
                let Some(target) = &job.target else {
                    return Ok(true);
                };
                let (dom, node) = target.current();
                if !Rc::ptr_eq(&dom, &job.owner.dom())
                    || dom.borrow().node_document(node).ok().flatten()
                        != Some(job.owner.document_root())
                {
                    return Ok(true);
                }
                if job.publication.is_none() {
                    if queue.borrow().seen.get(&(job.environment.id, node)) != Some(&job.key) {
                        return Ok(true);
                    }
                    job.publication = Some(self.start_script_completion_publication(
                        node,
                        job.result.unwrap(),
                        false,
                    )?)
                }
                if !self.resume_native_history_publication(job.publication.unwrap())? {
                    return Ok(false);
                }
                job.publication = None;
                Ok(true)
            })();
            match result {
                Ok(true) => complete += 1,
                Err(
                    error @ (HostError::LimitExceeded { .. }
                    | HostError::ScriptLoad(
                        crate::script_load_failure::ScriptLoadFailure::Capacity(_),
                    )),
                ) => {
                    if self.contexts.contains_key(&previous) {
                        self.active_realm = previous
                    }
                    return Err(error);
                }
                Err(HostError::ScriptLoad(
                    crate::script_load_failure::ScriptLoadFailure::Cancelled,
                )) => {}
                other => {
                    if let Err(error) = other
                        && job.first_error.is_none()
                    {
                        job.first_error = Some(error.to_string().chars().take(4096).collect());
                    }
                    job.retry = Instant::now() + Duration::from_millis(10);
                    queue.borrow_mut().jobs.push_back(job);
                }
            }
        }
        if self.contexts.contains_key(&previous) {
            self.active_realm = previous
        }
        Ok(complete)
    }
    fn cache_preloaded_module(&mut self, key: &ModulePreloadKey) -> Result<(), HostError> {
        let context = self
            .contexts
            .get(&self.active_realm)
            .cloned()
            .ok_or_else(|| HostError::Execution("preload realm retired".into()))?;
        let _deadline = ExecutionDeadline::arm(&self.isolate, self.execution_wall_limit);
        v8::scope!(let scope,&mut self.isolate);
        let context = v8::Local::new(scope, context);
        let scope = &mut v8::ContextScope::new(scope, context);
        v8::tc_scope!(let scope,scope);
        let kind = key.kind.expect("valid preload kind");
        if kind != crate::ModuleType::JavaScript {
            context.set_slot(Rc::new(PreparedImportLink {
                root: RefCell::new(Some(key.url.clone())),
                error: None,
            }));
            let _ = take_module_preparation_failure(scope);
            let module = load_typed_module_record(
                scope,
                &key.base,
                &key.url,
                kind,
                key.include,
                &key.policy,
            );
            context.remove_slot::<PreparedImportLink>();
            if let Some(failure) = take_module_preparation_failure(scope) {
                if scope.has_caught() {
                    scope.reset();
                }
                return Err(HostError::ScriptLoad(failure));
            }
            if module.is_none() {
                let environment = current_environment(scope).map_err(HostError::Execution)?;
                let retained_parse = scope.get_slot::<TypedModules>().is_some_and(|typed| {
                    matches!(
                        typed.entries.get(&(environment.id, key.url.clone(), kind)),
                        Some(Some(Err(_)))
                    )
                });
                if scope.is_execution_terminating() {
                    scope.cancel_terminate_execution();
                    scope.reset();
                    return Err(HostError::Execution(
                        "preload typed compilation interrupted".into(),
                    ));
                }
                scope.reset();
                if retained_parse {
                    return Ok(());
                }
                return Err(HostError::Execution(
                    "preload typed module allocation".into(),
                ));
            }
            return Ok(());
        }
        let environment = current_environment(scope).map_err(HostError::Execution)?;
        let cache_key = (environment.id, key.url.clone());
        if scope
            .get_slot::<ModuleCache>()
            .is_some_and(|cache| cache.0.contains_key(&cache_key))
        {
            return Ok(());
        }
        let response = module_fetch_results(scope)
            .borrow()
            .0
            .get(&key.url)
            .cloned()
            .flatten()
            .ok_or_else(|| HostError::Execution("preload response absent".into()))?;
        let error_key = Self::module_graph_key(&response.0, &response.1);
        if module_graph_errors(scope)
            .borrow()
            .parsed
            .contains_key(&error_key)
        {
            return Ok(());
        }
        let mut source = module_source(scope, &response.0, &response.1)?;
        let Some(module) = v8::script_compiler::compile_module(scope, &mut source) else {
            if scope.is_execution_terminating() {
                scope.cancel_terminate_execution();
            }
            scope.reset();
            return Err(HostError::Execution(
                "preload module compilation interrupted".into(),
            ));
        };
        retain_module_credentials(scope, module, &response.0);
        retain_module_name(scope, module, response.0);
        let module = v8::Global::new(scope, module);
        scope
            .get_slot_mut::<ModuleCache>()
            .ok_or_else(|| HostError::Execution("module cache absent".into()))?
            .0
            .insert(cache_key, module);
        Ok(())
    }
}

// Split Link field values without treating quoted commas/semicolons or URI
// punctuation as delimiters. Malformed links are ignored, never URL-guessed.
fn link_parts(value: &str, separator: char) -> Vec<&str> {
    let mut result = Vec::new();
    let (mut start, mut quoted, mut escaped, mut angle) = (0, false, false, false);
    for (index, ch) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if quoted && ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            quoted = !quoted;
            continue;
        }
        if !quoted {
            if ch == '<' {
                angle = true
            } else if ch == '>' {
                angle = false
            } else if ch == separator && !angle {
                result.push(&value[start..index]);
                start = index + ch.len_utf8();
            }
        }
    }
    if !quoted && !angle {
        result.push(&value[start..]);
    }
    result
}
fn link_parameter(value: &str) -> Option<String> {
    let value = value.trim();
    if !value.starts_with('"') {
        return Some(value.to_owned());
    }
    if !value.ends_with('"') || value.len() < 2 {
        return None;
    }
    let mut result = String::new();
    let mut escaped = false;
    for ch in value[1..value.len() - 1].chars() {
        if escaped {
            result.push(ch);
            escaped = false
        } else if ch == '\\' {
            escaped = true
        } else if ch == '"' {
            return None;
        } else {
            result.push(ch)
        }
    }
    (!escaped).then_some(result)
}
fn module_preload_header_keys(
    headers: &[(String, String)],
    base: &str,
    default_policy: &str,
) -> Result<Vec<ModulePreloadKey>, HostError> {
    let mut keys = Vec::new();
    let mut bytes = 0usize;
    for (name, value) in headers {
        if !name.eq_ignore_ascii_case("link") {
            continue;
        }
        bytes = bytes.saturating_add(value.len());
        if bytes > 192 * 1024 {
            return Err(HostError::LimitExceeded {
                resource: "modulepreload header bytes".into(),
            });
        }
        for link in link_parts(value, ',') {
            let parts = link_parts(link, ';');
            let Some(target) = parts
                .first()
                .map(|value| value.trim())
                .filter(|value| value.starts_with('<') && value.ends_with('>'))
            else {
                continue;
            };
            let mut params = BTreeMap::new();
            for parameter in &parts[1..] {
                let (key, value) = parameter
                    .trim()
                    .split_once('=')
                    .unwrap_or((parameter.trim(), ""));
                if let Some(value) = link_parameter(value) {
                    params
                        .entry(key.trim().to_ascii_lowercase())
                        .or_insert(value);
                }
            }
            if !params.get("rel").is_some_and(|rel| {
                rel.split_ascii_whitespace()
                    .any(|token| token.eq_ignore_ascii_case("modulepreload"))
            }) {
                continue;
            }
            // A link anchor changes the link context; document preload processing only
            // consumes links whose context remains this representation.
            if params.get("anchor").is_some_and(|anchor| {
                Url::parse(base)
                    .and_then(|base| base.join(anchor))
                    .map_or(true, |anchor| anchor.as_str() != base)
            }) {
                continue;
            }
            let Ok(url) = Url::parse(base).and_then(|base| base.join(&target[1..target.len() - 1]))
            else {
                continue;
            };
            let kind = match params
                .get("as")
                .map(String::as_str)
                .unwrap_or("")
                .to_ascii_lowercase()
                .as_str()
            {
                "" | "script" | "worker" | "sharedworker" | "serviceworker" | "audioworklet"
                | "paintworklet" => Some(crate::ModuleType::JavaScript),
                "json" => Some(crate::ModuleType::Json),
                "style" => Some(crate::ModuleType::Css),
                "text" => Some(crate::ModuleType::Text),
                _ => None,
            };
            if keys.len() >= 256 {
                return Err(HostError::LimitExceeded {
                    resource: "modulepreload header links".into(),
                });
            }
            keys.push(ModulePreloadKey {
                url: url.to_string(),
                kind,
                include: params
                    .get("crossorigin")
                    .is_some_and(|value| value.eq_ignore_ascii_case("use-credentials")),
                policy: params
                    .get("referrerpolicy")
                    .filter(|value| !value.is_empty())
                    .cloned()
                    .unwrap_or_else(|| default_policy.into()),
                integrity: params
                    .get("integrity")
                    .filter(|value| !value.is_empty())
                    .cloned(),
                base: base.into(),
                media: params.get("media").cloned().unwrap_or_default(),
                nonce: None,
                parser_inserted: true,
            });
        }
    }
    Ok(keys)
}
impl RawV8Host {
    pub(crate) fn prepare_module_preload_headers(
        &mut self,
        headers: &[(String, String)],
    ) -> Result<(), HostError> {
        let Some(state) = self.binding_states.get(&self.active_realm).cloned() else {
            return Ok(());
        };
        let context = self
            .contexts
            .get(&self.active_realm)
            .cloned()
            .ok_or_else(|| HostError::Execution("preload header context absent".into()))?;
        v8::scope!(let scope,&mut self.isolate);
        let context = v8::Local::new(scope, context);
        let scope = &mut v8::ContextScope::new(scope, context);
        capture_general_preload_headers(scope, &state, headers)?;
        let base = state
            .request_initiator_url()
            .map_err(|error| HostError::Execution(error.to_string()))?;
        let keys = module_preload_header_keys(headers, &base, &state.document_referrer_policy())?;
        let environment = current_environment(scope).map_err(HostError::Execution)?;
        let queue = module_preloads(scope);
        for mut key in keys {
            if !key.media.is_empty()
                && !state
                    .native_media_matches(&key.media)
                    .map_err(HostError::Execution)?
            {
                continue;
            }
            let identity = (
                environment.id,
                format!(
                    "{}\n{:?}\n{}\n{}\n{:?}\n{}",
                    key.url, key.kind, key.include, key.policy, key.integrity, key.media
                ),
            );
            if queue.borrow().headers.contains(&identity) {
                continue;
            }
            if queue.borrow().headers.len() >= 4096 {
                return Err(HostError::LimitExceeded {
                    resource: "modulepreload header identities".into(),
                });
            }
            if key.integrity.is_none() {
                key.integrity = module_import_map(scope)
                    .borrow()
                    .integrity_for(&key.url)
                    .map(str::to_owned)
            }
            admit_module_preload(scope, &state, None, key)?;
            queue.borrow_mut().headers.insert(identity);
        }
        Ok(())
    }
}

#[cfg(test)]
thread_local! {static FAIL_PRELOAD_PUBLICATION:std::cell::Cell<bool>=const{std::cell::Cell::new(false)};}
#[cfg(test)]
mod module_preload_tests {
    use super::*;
    fn host() -> RawV8Host {
        let mut host = RawV8Host::new_with_base_url_and_heap_limit(
            Url::parse("https://example.test/").unwrap(),
            256 * 1024 * 1024,
        );
        host.install_dom(
            Rc::new(RefCell::new(
                oryn_dom::html::parse("<!doctype html><head></head><body></body>")
                    .into_runtime()
                    .unwrap(),
            )),
            1,
        )
        .unwrap();
        host.commit_location("https://example.test/").unwrap();
        host
    }
    fn drain(host: &mut RawV8Host) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let queue = host
                .isolate
                .get_slot::<Rc<RefCell<ModulePreloads>>>()
                .unwrap()
                .clone();
            if queue.borrow().jobs.is_empty() {
                break;
            }
            for job in &mut queue.borrow_mut().jobs {
                job.retry = Instant::now()
            }
            host.poll_module_preloads().unwrap();
            assert!(Instant::now() < deadline, "preload failed to settle");
            std::thread::yield_now();
        }
    }
    #[test]
    fn parser_checkpoint_holds_baseline_graph_until_import_map_restore() {
        let mut host = host();
        host.register_import_map(r#"{"imports":{"child":"./baseline.js"}}"#)
            .unwrap();
        host.set_module_sources(&BTreeMap::from([
            (
                "https://example.test/main.js".into(),
                "import 'child';export const value=42;".into(),
            ),
            (
                "https://example.test/baseline.js".into(),
                "export const baseline=true;".into(),
            ),
        ]));
        host.prepare_module_preload_headers(&[(
            "link".into(),
            "</main.js>; rel=modulepreload".into(),
        )])
        .unwrap();
        let queue = host
            .isolate
            .get_slot::<Rc<RefCell<ModulePreloads>>>()
            .unwrap()
            .clone();
        assert_eq!(queue.borrow().jobs.len(), 1);
        let snapshot = host.capture_parser_module_snapshot().unwrap();
        host.register_import_map(r#"{"imports":{"provisional":"./wrong.js"}}"#)
            .unwrap();
        assert!(host.module_preload_delay().is_none());
        assert_eq!(host.poll_module_preloads().unwrap(), 0);
        assert_eq!(queue.borrow().jobs.len(), 1);
        assert_eq!(
            queue
                .borrow()
                .jobs
                .front()
                .unwrap()
                .preparation_pauses
                .get(),
            1
        );
        host.restore_parser_module_snapshot(&snapshot).unwrap();
        assert_eq!(
            queue
                .borrow()
                .jobs
                .front()
                .unwrap()
                .preparation_pauses
                .get(),
            0
        );
        drain(&mut host);
        assert!(queue.borrow().jobs.is_empty());
        assert!(
            host.isolate
                .get_slot::<ModuleCache>()
                .unwrap()
                .0
                .keys()
                .any(|(_, url)| url == "https://example.test/main.js")
        );
        assert_eq!(
            host.retained_module_fetch("https://example.test/baseline.js")
                .flatten()
                .map(|(_, source)| source),
            Some("export const baseline=true;".into())
        );
        // Dependencies are retained fetched responses until a later import links
        // them. Empty the fixture backend to prove that import reuses those bytes.
        host.set_module_sources(&BTreeMap::new());
        host.execute("import('/main.js').then(m=>globalThis.preloadedValue=m.value)")
            .unwrap();
        dynamic_import_continuation_tests::settle(&mut host);
        assert_eq!(
            host.evaluate_string("String(preloadedValue)").unwrap(),
            "42"
        );
    }
    #[test]
    fn parser_checkpoint_drop_releases_baseline_preparation_without_restore() {
        let mut host = host();
        host.set_module_sources(&BTreeMap::from([(
            "https://example.test/main.js".into(),
            "export {};".into(),
        )]));
        host.prepare_module_preload_headers(&[(
            "link".into(),
            "</main.js>; rel=modulepreload".into(),
        )])
        .unwrap();
        let snapshot = host.capture_parser_module_snapshot().unwrap();
        assert_eq!(host.poll_module_preloads().unwrap(), 0);
        drop(snapshot);
        assert!(host.module_preload_delay().is_some());
        drain(&mut host);
    }
    #[test]
    fn html_modulepreload_query_uses_document_encoding_but_headers_and_imports_use_utf8() {
        let mut host = host();
        let state = host.binding_states[&host.active_realm].clone();
        let dom = state.dom();
        dom.borrow_mut()
            .arena_mut()
            .get_mut(state.document_root())
            .unwrap()
            .document_metadata
            .as_mut()
            .unwrap()
            .encoding = "windows-1252".into();
        host.set_module_sources(&BTreeMap::from([
            (
                "https://example.test/element?%E5".into(),
                "import './child?å';export {};".into(),
            ),
            (
                "https://example.test/child?%C3%A5".into(),
                "export {};".into(),
            ),
            (
                "https://example.test/header?%C3%A5".into(),
                "export {};".into(),
            ),
        ]));
        host.prepare_module_preload_headers(&[(
            "link".into(),
            "</header?å>; rel=modulepreload".into(),
        )])
        .unwrap();
        host.execute("const link=document.createElement('link');link.rel='modulepreload';link.href='/element?å';document.head.append(link)").unwrap();
        drain(&mut host);
        let cache = host.isolate.get_slot::<ModuleCache>().unwrap();
        for url in [
            "https://example.test/element?%E5",
            "https://example.test/header?%C3%A5",
        ] {
            assert!(cache.0.keys().any(|(_, key)| key == url), "{url}");
        }
        assert!(
            !cache
                .0
                .keys()
                .any(|(_, url)| url == "https://example.test/child?%E5"
                    || url == "https://example.test/header?%E5")
        );
        assert!(
            host.retained_module_fetch("https://example.test/child?%C3%A5")
                .flatten()
                .is_some()
        );
        assert!(
            host.retained_module_fetch("https://example.test/child?%E5")
                .is_none()
        );
        host.set_module_sources(&BTreeMap::new());
        host.execute("import('/element?%E5').then(()=>globalThis.preloadedEncodingLinked=true)")
            .unwrap();
        dynamic_import_continuation_tests::settle(&mut host);
        assert_eq!(
            host.evaluate_string("String(preloadedEncodingLinked)")
                .unwrap(),
            "true"
        );
    }
    #[test]
    fn native_link_admits_once_and_compiles_without_evaluating() {
        let mut host = host();
        host.set_module_sources(&BTreeMap::from([(
            "https://example.test/main.js".into(),
            "globalThis.ran=(globalThis.ran||0)+1;export const value=42;".into(),
        )]));
        host.execute("globalThis.events=[];globalThis.link=document.createElement('link');link.rel='modulepreload';link.href='/main.js';link.onload=e=>events.push(e.isTrusted?'load':'untrusted');document.head.append(link)").unwrap();
        drain(&mut host);
        assert_eq!(
            host.evaluate_string("typeof ran+'|'+events.join()")
                .unwrap(),
            "undefined|load"
        );
        assert_eq!(host.isolate.get_slot::<ModuleCache>().unwrap().0.len(), 1);
        host.execute_module(
            "https://example.test/consumer.js",
            "import {value} from './main.js';globalThis.answer=value;",
            &BTreeMap::from([(
                "https://example.test/main.js".into(),
                "throw Error('replaced preload');".into(),
            )]),
        )
        .unwrap();
        assert_eq!(host.evaluate_string("ran+'|'+answer").unwrap(), "1|42");
    }
    #[test]
    fn preloaded_parse_error_fires_load_without_global_error() {
        let mut host = host();
        host.set_module_sources(&BTreeMap::from([(
            "https://example.test/bad.js".into(),
            "export const bad = ;".into(),
        )]));
        host.execute("globalThis.trace=[];onerror=()=>trace.push('global');let link=document.createElement('link');link.rel='modulepreload';link.href='/bad.js';link.onload=()=>trace.push('load');link.onerror=()=>trace.push('error');document.head.append(link)").unwrap();
        drain(&mut host);
        assert_eq!(host.evaluate_string("trace.join()").unwrap(), "load");
    }
    #[test]
    fn text_preload_keeps_typed_identity_and_string_payload() {
        let mut host = host();
        let url = "https://example.test/text";
        host.retain_typed_module_fetch(
            url,
            crate::ModuleType::Text,
            Some((url.into(), "not JavaScript;\n".into())),
        )
        .unwrap();
        host.execute("globalThis.loaded=false;let link=document.createElement('link');link.rel='modulepreload';link.as='text';link.href='/text';link.onload=()=>loaded=true;document.head.append(link)").unwrap();
        drain(&mut host);
        host.execute_module(
            "https://example.test/consumer.js",
            "import text from './text' with {type:'text'};globalThis.text=text;",
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(
            host.evaluate_string("String(loaded)+'|'+text").unwrap(),
            "true|not JavaScript;\n"
        );
    }
    #[test]
    fn publication_failure_retains_fetched_root_without_duplicate_fetch_or_event() {
        let mut host = host();
        host.set_module_sources(&BTreeMap::from([(
            "https://example.test/retry.js".into(),
            "export {};".into(),
        )]));
        host.execute("globalThis.events=0;let link=document.createElement('link');link.rel='modulepreload';link.href='/retry.js';link.onload=()=>events++;document.head.append(link)").unwrap();
        FAIL_PRELOAD_PUBLICATION.with(|fail| fail.set(true));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            host.poll_module_preloads().unwrap();
            if !FAIL_PRELOAD_PUBLICATION.with(|fail| fail.get()) {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(host.evaluate_string("String(events)").unwrap(), "0");
        let queue = host
            .isolate
            .get_slot::<Rc<RefCell<ModulePreloads>>>()
            .unwrap()
            .clone();
        assert_eq!(queue.borrow().jobs.front().unwrap().result, Some(true));
        drain(&mut host);
        assert_eq!(host.evaluate_string("String(events)").unwrap(), "1");
        assert_eq!(host.poll_module_preloads().unwrap(), 0);
    }
    #[test]
    fn publication_deadline_cancels_custody_once_without_synthetic_event() {
        let mut host = host();
        host.execute("globalThis.events=0;let link=document.createElement('link');link.rel='modulepreload';link.as='image';link.href='/never';link.onerror=()=>events++;document.head.append(link)").unwrap();
        FAIL_PRELOAD_PUBLICATION.with(|fail| fail.set(true));
        host.poll_module_preloads().unwrap();
        let queue = host
            .isolate
            .get_slot::<Rc<RefCell<ModulePreloads>>>()
            .unwrap()
            .clone();
        assert_eq!(queue.borrow().jobs.len(), 1);
        queue
            .borrow_mut()
            .jobs
            .front_mut()
            .unwrap()
            .publication_deadline = Some(Instant::now());
        assert!(
            matches!(host.poll_module_preloads(),Err(HostError::Execution(message))if message.contains("injected preload publication failure"))
        );
        assert!(queue.borrow().jobs.is_empty());
        assert!(host.module_preload_delay().is_none());
        assert_eq!(host.poll_module_preloads().unwrap(), 0);
        assert_eq!(host.evaluate_string("String(events)").unwrap(), "0");
    }
    #[test]
    fn fetch_deadline_fires_link_error_after_paused_preparation_releases() {
        let mut host = host();
        host.set_module_sources(&BTreeMap::from([(
            "https://example.test/timeout.js".into(),
            "export {};".into(),
        )]));
        host.execute("globalThis.trace=[];let link=document.createElement('link');link.rel='modulepreload';link.href='/timeout.js';link.onerror=()=>trace.push('error');link.onload=()=>trace.push('load');document.head.append(link)").unwrap();
        let queue = host
            .isolate
            .get_slot::<Rc<RefCell<ModulePreloads>>>()
            .unwrap()
            .clone();
        let pause = queue.borrow().jobs.front().unwrap().pause_preparation();
        queue.borrow_mut().jobs.front_mut().unwrap().deadline = Instant::now();
        assert!(host.module_preload_delay().is_none());
        assert_eq!(host.poll_module_preloads().unwrap(), 0);
        assert!(queue.borrow().jobs.front().unwrap().result.is_none());
        assert_eq!(host.evaluate_string("trace.join()").unwrap(), "");
        drop(pause);
        drain(&mut host);
        assert_eq!(host.evaluate_string("trace.join()").unwrap(), "error");
        assert_eq!(host.poll_module_preloads().unwrap(), 0);
    }
    #[test]
    fn rel_list_is_native_live_and_starts_preload_on_token_mutation() {
        let mut host = host();
        host.set_module_sources(&BTreeMap::from([(
            "https://example.test/token.js".into(),
            "export {};".into(),
        )]));
        host.execute("globalThis.events=0;globalThis.link=document.createElement('link');link.href='/token.js';link.onload=()=>events++;document.head.append(link);globalThis.tokens=link.relList;tokens.add('modulepreload');globalThis.check=tokens===link.relList&&tokens.length===1&&tokens[0]==='modulepreload'&&tokens.supports('modulepreload')&&link.classList!==tokens;").unwrap();
        drain(&mut host);
        assert_eq!(
            host.evaluate_string("check+'|'+events+'|'+link.rel")
                .unwrap(),
            "true|1|modulepreload"
        );
    }
    #[test]
    fn header_parser_preserves_quoted_delimiters_and_typed_options() {
        let keys=module_preload_header_keys(&[("Link".into(),"</a,b.js>; rel=\"alternate modulepreload\"; crossorigin=use-credentials; integrity=\"sha256-a;b,c\", </data>; rel=modulepreload; as=text, </ignored>; rel=preload".into())],"https://example.test/index","no-referrer").unwrap();
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].url, "https://example.test/a,b.js");
        assert_eq!(keys[0].integrity.as_deref(), Some("sha256-a;b,c"));
        assert!(keys[0].include);
        assert_eq!(keys[1].kind, Some(crate::ModuleType::Text));
        assert_eq!(keys[1].policy, "no-referrer");
    }
    #[cfg(unix)]
    #[test]
    fn header_admission_is_idempotent_and_parked_jobs_have_no_runnable_deadline() {
        let mut host = host();
        let headers = vec![("link".into(), "</x.js>;rel=modulepreload".into())];
        host.set_module_sources(&BTreeMap::from([(
            "https://example.test/x.js".into(),
            "export {};".into(),
        )]));
        host.prepare_module_preload_headers(&headers).unwrap();
        host.prepare_module_preload_headers(&headers).unwrap();
        let queue = host
            .isolate
            .get_slot::<Rc<RefCell<ModulePreloads>>>()
            .unwrap()
            .clone();
        assert_eq!(queue.borrow().jobs.len(), 1);
        let state = host.binding_states[&host.active_realm].clone();
        state.set_execution_suspended(true);
        assert!(state.execution_suspended());
        assert!(host.module_preload_delay().is_none());
        assert_eq!(host.poll_module_preloads().unwrap(), 0);
        assert_eq!(queue.borrow().jobs.len(), 1);
        state.set_execution_suspended(false);
        assert!(!state.execution_suspended());
        assert!(host.module_preload_delay().is_some());
        drain(&mut host);
        assert!(queue.borrow().jobs.is_empty());
    }
}
