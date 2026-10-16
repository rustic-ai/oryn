
/// Inputs retained together for local preload key.
 struct LocalPreloadKeyInput<'a> {
     state: &'a V8BindingState,
     backend: &'a ModuleGraphBackend,
     kind: &'a str,
     url: String,
     integrity: Option<String>,
     cross_origin: Option<String>,
     policy: String,
     nonce: Option<String>,
     parser_inserted: bool,
}
// Classic-script and stylesheet preload admission through native source loaders.
// Included in raw_v8; network producer ownership is independent of link events.
#[derive(Default)]
struct GeneralPreloads {
    jobs: std::collections::VecDeque<GeneralPreload>,
    seen: BTreeMap<(EnvironmentId, NodeId), String>,
    headers: BTreeSet<(EnvironmentId, String)>,
    owners: BTreeMap<EnvironmentId, (std::rc::Weak<V8BindingState>, u64)>,
}
struct GeneralPreload {
    preparation_pauses: Rc<std::cell::Cell<usize>>,
    owner: Rc<V8BindingState>,
    environment: EnvironmentDescriptor,
    epoch: u64,
    cancellation: u64,
    target: Option<oryn_dom::TrackedNodeLease>,
    key: String,
    source: GeneralPreloadSource,
    publication: Option<u64>,
    result: Option<bool>,
    first_error: Option<String>,
    retry: Instant,
    deadline: Instant,
}
enum GeneralPreloadSource {
    #[cfg(unix)]
    Parent(std::sync::Arc<crate::context_service::PreparedResourcePreload>),
    Local(std::sync::Arc<crate::local_resource_preload::Entry>),
}
impl GeneralPreloadSource {
    fn cancel(&self) {
        match self {
            #[cfg(unix)]
            Self::Parent(source) => source.cancel(),
            Self::Local(source) => source.cancel(),
        }
    }
    fn result(&self) -> Option<Result<bool, HostError>> {
        use crate::script_load_failure::ScriptLoadFailure;
        let outcome = match self {
            #[cfg(unix)]
            Self::Parent(source) => source.outcome().map(|result| {
                result.and_then(|response| match response {
                    crate::context_service::ContextServiceResponse::ScriptLoadFailed {
                        failure,
                    } => Err(failure),
                    _ => Ok(()),
                })
            }),
            Self::Local(source) => source.result().map(|result| result.map(|_| ())),
        };
        outcome.map(|result| match result {
            Ok(()) => Ok(true),
            Err(ScriptLoadFailure::Fetch(_) | ScriptLoadFailure::Integrity(_)) => Ok(false),
            Err(failure) => Err(HostError::ScriptLoad(failure)),
        })
    }
}
struct GeneralPreloadPause(Rc<std::cell::Cell<usize>>);
impl Drop for GeneralPreloadPause {
    fn drop(&mut self) {
        self.0.set(self.0.get() - 1);
    }
}
impl GeneralPreload {
    fn pause_preparation(&self) -> GeneralPreloadPause {
        self.preparation_pauses
            .set(self.preparation_pauses.get() + 1);
        GeneralPreloadPause(self.preparation_pauses.clone())
    }
}
impl Drop for GeneralPreload {
    fn drop(&mut self) {
        if let Some(receipt) = self.publication.take() {
            oryn_js::navigation_api::cancel_native_history_receipt(&self.owner, receipt);
        }
    }
}
fn general_preloads(scope: &mut v8::PinScope) -> Rc<RefCell<GeneralPreloads>> {
    if let Some(queue) = scope.get_slot::<Rc<RefCell<GeneralPreloads>>>() {
        let queue = queue.clone();
        {
            let mut registry = queue.borrow_mut();
            let dead = registry
                .owners
                .iter()
                .filter_map(|(id, (owner, epoch))| {
                    owner
                        .upgrade()
                        .is_none_or(|state| {
                            state.execution_retired() || state.document_authority_epoch() != *epoch
                        })
                        .then_some(*id)
                })
                .collect::<BTreeSet<_>>();
            registry.seen.retain(|(id, _), _| !dead.contains(id));
            registry.headers.retain(|(id, _)| !dead.contains(id));
            registry.owners.retain(|id, _| !dead.contains(id));
        }
        return queue;
    }
    let queue = Rc::new(RefCell::new(GeneralPreloads::default()));
    scope.set_slot(queue.clone());
    queue
}
fn capture_general_preloads(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    root: NodeId,
    parser_inserted: bool,
) -> Result<(), oryn_js::v8_bindings::V8BindingError> {
    #[cfg(not(unix))]
    {
        let backend = module_fetch_backend_in_scope(scope).map_err(|error| {
            oryn_js::v8_bindings::V8BindingError::EventCallback(error.to_string())
        })?;
        capture_local_general_preloads(scope, state, root, parser_inserted, backend)
    }
    #[cfg(unix)]
    {
        use crate::context_service::{
            ContextServiceRequest as Request, ResourceInitiator, StylesheetRequest,
        };
        use oryn_js::v8_bindings::V8BindingError;
        if state.execution_retired() || state.document_load_stopped() {
            return Ok(());
        }
        let dom = state.dom();
        let (nodes, inactive) = {
            let dom = dom.borrow();
            let mut nodes = Vec::new();
            let mut inactive = Vec::new();
            for node in dom.arena().descendants_inclusive(root)? {
                if !dom.is_element(node, "http://www.w3.org/1999/xhtml", "link") {
                    continue;
                }
                if dom.is_connected(node)?
                    && dom
                        .attribute_in_null_namespace(node, "rel")?
                        .is_some_and(|rel| {
                            rel.split_ascii_whitespace()
                                .any(|token| token.eq_ignore_ascii_case("preload"))
                        })
                    && dom
                        .attribute_in_null_namespace(node, "as")?
                        .is_some_and(|kind| {
                            kind.eq_ignore_ascii_case("script")
                                || kind.eq_ignore_ascii_case("style")
                        })
                {
                    nodes.push(node)
                } else {
                    inactive.push(node)
                }
            }
            (nodes, inactive)
        };
        // A link ceasing to be a preload resets its element preparation identity;
        // the original network producer remains independently owned by the store.
        if !inactive.is_empty() {
            let environment = current_environment(scope).map_err(V8BindingError::EventCallback)?;
            if let Some(queue) = scope.get_slot::<Rc<RefCell<GeneralPreloads>>>() {
                let mut queue = queue.borrow_mut();
                queue.seen.retain(|(owner, node), _| {
                    *owner != environment.id || !inactive.contains(node)
                });
            }
        }
        if nodes.is_empty() {
            return Ok(());
        }
        let backend = module_fetch_backend_in_scope(scope)
            .map_err(|error| V8BindingError::EventCallback(error.to_string()))?;
        let ModuleGraphBackend::Window(client, page, Some((creator, initiator)), script_policy) =
            backend
        else {
            return capture_local_general_preloads(scope, state, root, parser_inserted, backend);
        };
        let environment = current_environment(scope).map_err(V8BindingError::EventCallback)?;
        let queue = general_preloads(scope);
        for node in nodes {
            let values = {
                let dom = dom.borrow();
                if !dom.is_connected(node)?
                    || !dom.is_element(node, "http://www.w3.org/1999/xhtml", "link")
                {
                    continue;
                }
                let attr = |name| {
                    dom.attribute_in_null_namespace(node, name)
                        .map(|value| value.map(str::to_owned))
                };
                if !attr("rel")?.is_some_and(|rel| {
                    rel.split_ascii_whitespace()
                        .any(|token| token.eq_ignore_ascii_case("preload"))
                }) {
                    queue.borrow_mut().seen.remove(&(environment.id, node));
                    continue;
                }
                (
                    attr("as")?.unwrap_or_default().to_ascii_lowercase(),
                    attr("href")?.unwrap_or_default(),
                    attr("crossorigin")?,
                    attr("integrity")?,
                    attr("referrerpolicy")?,
                    attr("media")?.unwrap_or_default(),
                    attr("type")?.unwrap_or_default(),
                    dom.element_nonce_for_csp(node)?.map(str::to_owned),
                )
            };
            if !matches!(values.0.as_str(), "script" | "style") || values.1.is_empty() {
                queue.borrow_mut().seen.remove(&(environment.id, node));
                continue;
            }
            if !values.5.is_empty()
                && !oryn_js::v8_bindings::native_media_matches_in_scope(scope, state, &values.5)?
            {
                queue.borrow_mut().seen.remove(&(environment.id, node));
                continue;
            }
            if !values.6.is_empty()
                && if values.0 == "script" {
                    !oryn_dom::html::is_javascript_mime_type(&values.6)
                } else {
                    !values.6.eq_ignore_ascii_case("text/css")
                }
            {
                queue.borrow_mut().seen.remove(&(environment.id, node));
                continue;
            }
            let url = state
                .native_document_resource_url(&values.1)
                .map_err(|error| V8BindingError::EventCallback(error.to_string()))?
                .to_string();
            let policy = values
                .4
                .as_deref()
                .and_then(crate::referrer::ReferrerPolicy::parse_attribute)
                .map(|policy| policy.as_str().to_owned())
                .unwrap_or_else(|| state.document_referrer_policy());
            let (encoding, quirks) = {
                let dom = dom.borrow();
                let metadata = dom
                    .arena()
                    .get(state.document_root())?
                    .document_metadata
                    .as_ref();
                (
                    metadata
                        .map_or("UTF-8", |metadata| metadata.encoding.as_str())
                        .to_owned(),
                    metadata.is_some_and(|metadata| metadata.quirks),
                )
            };
            let request = if values.0 == "script" {
                Request::LoadScript {
                    fallback_encoding: Some(encoding),
                    parser_inserted,
                    script_policy: script_policy.clone(),
                    page_id: page,
                    creator: creator.clone(),
                    url,
                    module: false,
                    nonce: values.7,
                    integrity: values.3,
                    cross_origin: values.2,
                    initiator_url: Some(initiator.clone()),
                    referrer_policy: Some(policy),
                    referrer_source: None,
                }
            } else {
                Request::LoadPageResource {
                    download: None,
                    script_authority: None,
                    page_id: page,
                    initiator: Box::new(ResourceInitiator {
                        embedded_type: None,
                        request_state: None,
                        navigation: Default::default(),
                        creator: creator.clone(),
                        stylesheet: Some(StylesheetRequest {
                            policy: script_policy.clone(),
                            render_blocking: false,
                            quirks,
                            nonce: values.7,
                            integrity: values.3,
                            cross_origin: values.2,
                        }),
                        prepared_policy: None,
                    }),
                    form_authority: None,
                    url,
                    destination: "stylesheet".into(),
                    parent_encoding: Some(encoding),
                    initiator_url: Some(initiator.clone()),
                    referrer_policy: Some(policy),
                    post: None,
                }
            };
            let key = serde_json::to_string(&request)
                .map_err(|error| V8BindingError::EventCallback(error.to_string()))?;
            if queue.borrow().seen.get(&(environment.id, node)) == Some(&key) {
                continue;
            }
            if key.len() > 256 * 1024
                || queue.borrow().jobs.len() >= 256
                || queue.borrow().seen.len() >= 4096
            {
                return Err(V8BindingError::Allocation(
                    "general preload admission limit",
                ));
            }
            let target = oryn_dom::TrackedNodeLease::new(&dom, node)?;
            queue
                .borrow_mut()
                .jobs
                .try_reserve(1)
                .map_err(|_| V8BindingError::Allocation("general preload queue"))?;
            let source = client
                .begin_resource_preload(request)
                .map_err(|error| V8BindingError::EventCallback(error.to_string()))?;
            let mut queue = queue.borrow_mut();
            queue.owners.insert(
                environment.id,
                (Rc::downgrade(state), state.document_authority_epoch()),
            );
            queue.seen.insert((environment.id, node), key.clone());
            queue.jobs.push_back(GeneralPreload {
                preparation_pauses: Default::default(),
                owner: state.clone(),
                environment,
                epoch: state.document_authority_epoch(),
                cancellation: state.document_load_cancellation_epoch(),
                target: Some(target),
                key,
                source: GeneralPreloadSource::Parent(source),
                publication: None,
                result: None,
                first_error: None,
                retry: Instant::now(),
                deadline: Instant::now() + Duration::from_secs(70),
            });
        }
        Ok(())
    }
}
impl RawV8Host {
    fn general_preload_delay(&self) -> Option<Duration> {
        {
            let queue = self.isolate.get_slot::<Rc<RefCell<GeneralPreloads>>>()?;
            let now = Instant::now();
            queue
                .borrow()
                .jobs
                .iter()
                .filter_map(|job| {
                    if job.owner.execution_retired()
                        || job.owner.document_authority_epoch() != job.epoch
                    {
                        return Some(Duration::ZERO);
                    }
                    if job.owner.execution_suspended() || job.preparation_pauses.get() != 0 {
                        return None;
                    }
                    Some(job.retry.min(job.deadline).saturating_duration_since(now))
                })
                .min()
        }
    }
    fn poll_general_preloads(&mut self) -> Result<usize, HostError> {
        {
            let Some(queue) = self
                .isolate
                .get_slot::<Rc<RefCell<GeneralPreloads>>>()
                .cloned()
            else {
                return Ok(0);
            };
            let count = queue.borrow().jobs.len().min(32);
            let previous = self.active_realm;
            let mut completed = 0;
            for _ in 0..count {
                let Some(mut job) = queue.borrow_mut().jobs.pop_front() else {
                    break;
                };
                if job.owner.execution_retired()
                    || job.epoch != job.owner.document_authority_epoch()
                    || job.cancellation != job.owner.document_load_cancellation_epoch()
                {
                    job.source.cancel();
                    continue;
                }
                if job.preparation_pauses.get() != 0 {
                    queue.borrow_mut().jobs.push_back(job);
                    continue;
                }
                if Instant::now() >= job.deadline {
                    job.source.cancel();
                    let detail = job
                        .first_error
                        .take()
                        .unwrap_or_else(|| "preload completion remained pending".into());
                    drop(job);
                    if self.contexts.contains_key(&previous) {
                        self.active_realm = previous
                    }
                    return Err(HostError::Execution(format!(
                        "general preload publication deadline exceeded: {detail}"
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
                    job.source.cancel();
                    continue;
                }
                if job.result.is_none() {
                    let Some(result) = job.source.result() else {
                        job.retry = Instant::now() + Duration::from_millis(10);
                        queue.borrow_mut().jobs.push_back(job);
                        continue;
                    };
                    job.result = Some(match result {
                        Ok(success) => success,
                        Err(error) => {
                            job.source.cancel();
                            drop(job);
                            if self.contexts.contains_key(&previous) {
                                self.active_realm = previous
                            }
                            return Err(error);
                        }
                    });
                    job.deadline = Instant::now() + Duration::from_secs(10);
                }
                let Some(target) = &job.target else {
                    completed += 1;
                    continue;
                };
                let (dom, node) = target.current();
                if !Rc::ptr_eq(&dom, &job.owner.dom())
                    || dom.borrow().node_document(node).ok().flatten()
                        != Some(job.owner.document_root())
                {
                    continue;
                }
                self.active_realm = job.environment.realm;
                let outcome = (|| {
                    if job.publication.is_none() {
                        if queue.borrow().seen.get(&(job.environment.id, node)) != Some(&job.key) {
                            return Ok(true);
                        }
                        let success = job.result.unwrap();
                        job.publication =
                            Some(self.start_script_completion_publication(node, success, false)?)
                    }
                    if !self.resume_native_history_publication(job.publication.unwrap())? {
                        return Ok(false);
                    }
                    job.publication = None;
                    Ok::<_, HostError>(true)
                })();
                match outcome {
                    Ok(true) => completed += 1,
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
            Ok(completed)
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct GeneralPreloadHeader {
    url: String,
    kind: String,
    cross_origin: Option<String>,
    integrity: Option<String>,
    policy: String,
    media: String,
    mime: String,
}
fn general_preload_header_keys(
    headers: &[(String, String)],
    base: &str,
    default_policy: &str,
) -> Result<Vec<GeneralPreloadHeader>, HostError> {
    let mut keys = Vec::new();
    let mut bytes = 0usize;
    for (name, value) in headers {
        if !name.eq_ignore_ascii_case("link") {
            continue;
        }
        bytes = bytes.saturating_add(value.len());
        if bytes > 192 * 1024 {
            return Err(HostError::LimitExceeded {
                resource: "preload header bytes".into(),
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
                    .any(|token| token.eq_ignore_ascii_case("preload"))
            }) {
                continue;
            }
            let kind = params
                .get("as")
                .map(|value| value.to_ascii_lowercase())
                .unwrap_or_default();
            if !matches!(kind.as_str(), "script" | "style") {
                continue;
            }
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
            if keys.len() >= 256 {
                return Err(HostError::LimitExceeded {
                    resource: "preload header links".into(),
                });
            }
            keys.push(GeneralPreloadHeader {
                url: url.to_string(),
                kind,
                cross_origin: params.remove("crossorigin"),
                integrity: params.remove("integrity"),
                policy: params
                    .get("referrerpolicy")
                    .and_then(|value| crate::referrer::ReferrerPolicy::parse_attribute(value))
                    .map(|policy| policy.as_str().to_owned())
                    .unwrap_or_else(|| default_policy.into()),
                media: params.remove("media").unwrap_or_default(),
                mime: params.remove("type").unwrap_or_default(),
            });
        }
    }
    Ok(keys)
}
fn capture_general_preload_headers(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    headers: &[(String, String)],
) -> Result<(), HostError> {
    let base = state
        .request_initiator_url()
        .map_err(|error| HostError::Execution(error.to_string()))?;
    let keys = general_preload_header_keys(headers, &base, &state.document_referrer_policy())?;
    if keys.is_empty() {
        return Ok(());
    }
    #[cfg(not(unix))]
    {
        let backend = module_fetch_backend_in_scope(scope)?;
        capture_local_preload_headers(scope, state, keys, backend)
    }
    #[cfg(unix)]
    {
        use crate::context_service::{
            ContextServiceRequest as Request, ResourceInitiator, StylesheetRequest,
        };
        if state.execution_retired() || state.document_load_stopped() {
            return Ok(());
        }
        let backend = module_fetch_backend_in_scope(scope)?;
        let ModuleGraphBackend::Window(client, page, Some((creator, initiator)), script_policy) =
            backend
        else {
            return capture_local_preload_headers(scope, state, keys, backend);
        };
        let environment = current_environment(scope).map_err(HostError::Execution)?;
        let queue = general_preloads(scope);
        let (encoding, quirks) = {
            let dom = state.dom();
            let dom = dom.borrow();
            let metadata = dom
                .arena()
                .get(state.document_root())
                .map_err(|error| HostError::Execution(error.to_string()))?
                .document_metadata
                .as_ref();
            (
                metadata
                    .map_or("UTF-8", |metadata| metadata.encoding.as_str())
                    .to_owned(),
                metadata.is_some_and(|metadata| metadata.quirks),
            )
        };
        for key in keys {
            if !key.media.is_empty()
                && !oryn_js::v8_bindings::native_media_matches_in_scope(scope, state, &key.media)
                    .map_err(|error| HostError::Execution(error.to_string()))?
            {
                continue;
            }
            if !key.mime.is_empty()
                && if key.kind == "script" {
                    !oryn_dom::html::is_javascript_mime_type(&key.mime)
                } else {
                    !key.mime.eq_ignore_ascii_case("text/css")
                }
            {
                continue;
            }
            let request = if key.kind == "script" {
                Request::LoadScript {
                    fallback_encoding: Some(encoding.clone()),
                    parser_inserted: true,
                    script_policy: script_policy.clone(),
                    page_id: page,
                    creator: creator.clone(),
                    url: key.url,
                    module: false,
                    nonce: None,
                    integrity: key.integrity,
                    cross_origin: key.cross_origin,
                    initiator_url: Some(initiator.clone()),
                    referrer_policy: Some(key.policy),
                    referrer_source: None,
                }
            } else {
                Request::LoadPageResource {
                    download: None,
                    script_authority: None,
                    page_id: page,
                    initiator: Box::new(ResourceInitiator {
                        embedded_type: None,
                        request_state: None,
                        navigation: Default::default(),
                        creator: creator.clone(),
                        stylesheet: Some(StylesheetRequest {
                            policy: script_policy.clone(),
                            render_blocking: false,
                            quirks,
                            nonce: None,
                            integrity: key.integrity,
                            cross_origin: key.cross_origin,
                        }),
                        prepared_policy: None,
                    }),
                    form_authority: None,
                    url: key.url,
                    destination: "stylesheet".into(),
                    parent_encoding: Some(encoding.clone()),
                    initiator_url: Some(initiator.clone()),
                    referrer_policy: Some(key.policy),
                    post: None,
                }
            };
            let identity = serde_json::to_string(&request)
                .map_err(|error| HostError::Execution(error.to_string()))?;
            if queue
                .borrow()
                .headers
                .contains(&(environment.id, identity.clone()))
            {
                continue;
            }
            if identity.len() > 256 * 1024 || queue.borrow().headers.len() >= 4096 {
                return Err(HostError::LimitExceeded {
                    resource: "general preload header identities".into(),
                });
            }
            // Header preloads have no element event target. The document-owned store
            // retains the original producer through later consumption or cancellation.
            if queue.borrow().jobs.len() >= 256 {
                return Err(HostError::LimitExceeded {
                    resource: "preload pending headers".into(),
                });
            }
            queue
                .borrow_mut()
                .jobs
                .try_reserve(1)
                .map_err(|_| HostError::LimitExceeded {
                    resource: "preload header queue".into(),
                })?;
            let source = client
                .begin_resource_preload(request)
                .map_err(|error| HostError::Execution(error.to_string()))?;
            let mut queue = queue.borrow_mut();
            queue.owners.insert(
                environment.id,
                (Rc::downgrade(state), state.document_authority_epoch()),
            );
            queue.headers.insert((environment.id, identity.clone()));
            queue.jobs.push_back(GeneralPreload {
                preparation_pauses: Default::default(),
                owner: state.clone(),
                environment,
                epoch: state.document_authority_epoch(),
                cancellation: state.document_load_cancellation_epoch(),
                target: None,
                key: identity,
                source: GeneralPreloadSource::Parent(source),
                publication: None,
                result: None,
                first_error: None,
                retry: Instant::now(),
                deadline: Instant::now() + Duration::from_secs(70),
            });
        }
        Ok(())
    }
}
#[cfg(test)]
mod general_preload_header_tests {
    use super::*;
    #[test]
    fn native_header_inventory_preserves_destination_cors_integrity_and_url_rules() {
        let headers=vec![("Link".into(),"</a?å>; rel=preload; as=script; crossorigin; integrity=\"sha256-first\", </s.css>; rel=\"preload stylesheet\"; as=style; media=\"screen, print\"; referrerpolicy=no-referrer, </skip>; rel=preload; as=script; anchor=/elsewhere, </image>; rel=preload; as=image".into())];
        let keys =
            general_preload_header_keys(&headers, "https://example.test/page", "origin").unwrap();
        assert_eq!(keys.len(), 2);
        assert!(keys[0].url.ends_with("?%C3%A5"));
        assert_eq!(keys[0].cross_origin.as_deref(), Some(""));
        assert_eq!(keys[0].integrity.as_deref(), Some("sha256-first"));
        assert_eq!(keys[1].kind, "style");
        assert_eq!(keys[1].media, "screen, print");
        assert_eq!(keys[1].policy, "no-referrer");
    }
}

fn local_preload_key(params: LocalPreloadKeyInput<'_>) -> Result<crate::local_resource_preload::Key, HostError> {
        let LocalPreloadKeyInput { state, backend, kind, url, integrity, cross_origin, policy, nonce, parser_inserted } = params;

    let (encoding, quirks) = {
        let dom = state.dom();
        let dom = dom.borrow();
        let metadata = dom
            .arena()
            .get(state.document_root())
            .map_err(|error| HostError::Execution(error.to_string()))?
            .document_metadata
            .as_ref();
        (
            metadata
                .map_or("UTF-8", |metadata| metadata.encoding.as_str())
                .to_owned(),
            metadata.is_some_and(|metadata| metadata.quirks),
        )
    };
    if kind == "script" {
        let mut key = backend.classic_preload_key(crate::executable::module_graph_load::ClassicPreloadKeyInput { url: &url, integrity, cross_origin, policy: &policy, nonce, parser_inserted, encoding: Some(&encoding) })?;
        if matches!(backend, ModuleGraphBackend::Sources(..)) {
            key.initiator = state
                .request_initiator_url()
                .map_err(|error| HostError::Execution(error.to_string()))?;
        }
        return Ok(key);
    }
    Ok(crate::local_resource_preload::Key {
        destination: "stylesheet",
        url,
        policy: backend
            .local_script_policy()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| HostError::Execution(error.to_string()))?,
        initiator: state
            .request_initiator_url()
            .map_err(|error| HostError::Execution(error.to_string()))?,
        referrer_policy: policy,
        encoding,
        nonce,
        integrity,
        cross_origin,
        parser_inserted: true,
        quirks,
    })
}
fn begin_local_preload(
    scope: &mut v8::PinScope,
    backend: &ModuleGraphBackend,
    key: crate::local_resource_preload::Key,
) -> Result<std::sync::Arc<crate::local_resource_preload::Entry>, HostError> {
    let reports = backend
        .reports()
        .ok_or_else(|| HostError::Execution("standalone preload report owner absent".into()))?;
    let source = reports.3.admit(key.clone())?;
    if !source.start() {
        return Ok(source);
    }
    let request_id = if matches!(backend, ModuleGraphBackend::Broker { .. }) {
        let Some(fetch) = current_fetch_endpoint(scope) else {
            source.cancel();
            return Err(HostError::Execution(
                "standalone preload fetch owner absent".into(),
            ));
        };
        let next = fetch.next_script_request.get();
        let Some(end) = next.checked_add(11) else {
            source.cancel();
            return Err(HostError::LimitExceeded {
                resource: "preload request identifiers".into(),
            });
        };
        fetch.next_script_request.set(end);
        Some(next)
    } else {
        None
    };
    let backend = backend.clone();
    let pending = std::sync::Arc::downgrade(&source);
    if let Err(error) = crate::resource_io_executor::submit(move || {
        let Some(source) = pending.upgrade() else {
            return;
        };
        if source.cancelled() {
            return;
        }
        let mut trace = Vec::new();
        let mut hash_evidence = None;
        let result = if key.destination == "script" {
            backend
                .fetch_classic_uncached(crate::executable::module_graph_load::UncachedClassicFetchInput { requested: &key.url, initiator_url: &key.initiator, url: key.url.clone(), integrity: key.integrity, cross_origin: key.cross_origin, policy: key.referrer_policy, nonce: key.nonce, parser_inserted: key.parser_inserted, render_blocking: false, fallback_encoding: Some(&key.encoding), request_id, cancel: &source.cancel, preload: Some(source.status.clone()), trace: Some(&mut trace), hash_evidence: Some(&mut hash_evidence) })
                .map(|(url, text, muted, policy)| {
                    crate::local_resource_preload::Response::Script(url, text, muted, policy)
                })
        } else {
            crate::executable::standalone_stylesheet::preload(
                backend,
                key,
                request_id.unwrap_or(0),
                &source,
            )
        };
        if let Some(evidence) = hash_evidence {
            source.set_hash_evidence(evidence);
        }
        if !trace.is_empty() {
            source.set_trace(trace);
        }
        source.complete(result);
    }) {
        source.complete(Err(HostError::Execution(error)));
    }
    Ok(source)
}
fn capture_local_general_preloads(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    root: NodeId,
    parser_inserted: bool,
    backend: ModuleGraphBackend,
) -> Result<(), oryn_js::v8_bindings::V8BindingError> {
    use oryn_js::v8_bindings::V8BindingError;
    let error = |error: HostError| V8BindingError::EventCallback(error.to_string());
    if state.execution_retired() || state.document_load_stopped() {
        return Ok(());
    }
    let environment = current_environment(scope).map_err(V8BindingError::EventCallback)?;
    let queue = general_preloads(scope);
    let dom = state.dom();
    let nodes = dom.borrow().arena().descendants_inclusive(root)?;
    for node in nodes {
        let values = {
            let dom = dom.borrow();
            if !dom.is_element(node, "http://www.w3.org/1999/xhtml", "link") {
                continue;
            }
            let attr = |name| {
                dom.attribute_in_null_namespace(node, name)
                    .map(|value| value.map(str::to_owned))
            };
            if !dom.is_connected(node)?
                || !attr("rel")?.is_some_and(|value| {
                    value
                        .split_ascii_whitespace()
                        .any(|token| token.eq_ignore_ascii_case("preload"))
                })
            {
                queue.borrow_mut().seen.remove(&(environment.id, node));
                continue;
            }
            (
                attr("as")?.unwrap_or_default().to_ascii_lowercase(),
                attr("href")?.unwrap_or_default(),
                attr("crossorigin")?,
                attr("integrity")?,
                attr("referrerpolicy")?,
                attr("media")?.unwrap_or_default(),
                attr("type")?.unwrap_or_default(),
                dom.element_nonce_for_csp(node)?.map(str::to_owned),
            )
        };
        if !matches!(values.0.as_str(), "script" | "style")
            || values.1.is_empty()
            || (!values.5.is_empty()
                && !oryn_js::v8_bindings::native_media_matches_in_scope(scope, state, &values.5)?)
            || (!values.6.is_empty()
                && if values.0 == "script" {
                    !oryn_dom::html::is_javascript_mime_type(&values.6)
                } else {
                    !values.6.eq_ignore_ascii_case("text/css")
                })
        {
            queue.borrow_mut().seen.remove(&(environment.id, node));
            continue;
        }
        let url = state
            .native_document_resource_url(&values.1)
            .map_err(|error| V8BindingError::EventCallback(error.to_string()))?
            .to_string();
        let policy = values
            .4
            .as_deref()
            .and_then(crate::referrer::ReferrerPolicy::parse_attribute)
            .map(|policy| policy.as_str().to_owned())
            .unwrap_or_else(|| state.document_referrer_policy());
        let key = local_preload_key(LocalPreloadKeyInput { state, backend: &backend, kind: &values.0, url, integrity: values.3, cross_origin: values.2, policy, nonce: values.7, parser_inserted })
        .map_err(error)?;
        let identity = serde_json::to_string(&key)
            .map_err(|error| V8BindingError::EventCallback(error.to_string()))?;
        if queue.borrow().seen.get(&(environment.id, node)) == Some(&identity) {
            continue;
        }
        if queue.borrow().jobs.len() >= 256 || queue.borrow().seen.len() >= 4096 {
            return Err(V8BindingError::Allocation(
                "standalone preload admission limit",
            ));
        }
        let target = oryn_dom::TrackedNodeLease::new(&dom, node)?;
        queue
            .borrow_mut()
            .jobs
            .try_reserve(1)
            .map_err(|_| V8BindingError::Allocation("standalone preload event queue"))?;
        let source = begin_local_preload(scope, &backend, key).map_err(error)?;
        let mut queue = queue.borrow_mut();
        queue.owners.insert(
            environment.id,
            (Rc::downgrade(state), state.document_authority_epoch()),
        );
        queue.seen.insert((environment.id, node), identity.clone());
        queue.jobs.push_back(GeneralPreload {
            preparation_pauses: Default::default(),
            owner: state.clone(),
            environment,
            epoch: state.document_authority_epoch(),
            cancellation: state.document_load_cancellation_epoch(),
            target: Some(target),
            key: identity,
            source: GeneralPreloadSource::Local(source),
            publication: None,
            result: None,
            first_error: None,
            retry: Instant::now(),
            deadline: Instant::now() + Duration::from_secs(70),
        });
    }
    Ok(())
}
fn capture_local_preload_headers(
    scope: &mut v8::PinScope,
    state: &Rc<V8BindingState>,
    keys: Vec<GeneralPreloadHeader>,
    backend: ModuleGraphBackend,
) -> Result<(), HostError> {
    let environment = current_environment(scope).map_err(HostError::Execution)?;
    let queue = general_preloads(scope);
    for key in keys {
        if !key.media.is_empty()
            && !oryn_js::v8_bindings::native_media_matches_in_scope(scope, state, &key.media)
                .map_err(|error| HostError::Execution(error.to_string()))?
        {
            continue;
        }
        if !key.mime.is_empty()
            && if key.kind == "script" {
                !oryn_dom::html::is_javascript_mime_type(&key.mime)
            } else {
                !key.mime.eq_ignore_ascii_case("text/css")
            }
        {
            continue;
        }
        let key = local_preload_key(LocalPreloadKeyInput { state, backend: &backend, kind: &key.kind, url: key.url, integrity: key.integrity, cross_origin: key.cross_origin, policy: key.policy, nonce: None, parser_inserted: true })?;
        let identity =
            serde_json::to_string(&key).map_err(|error| HostError::Execution(error.to_string()))?;
        if queue
            .borrow()
            .headers
            .contains(&(environment.id, identity.clone()))
        {
            continue;
        }
        if queue.borrow().headers.len() >= 4096 {
            return Err(HostError::LimitExceeded {
                resource: "standalone preload header identities".into(),
            });
        }
        if queue.borrow().jobs.len() >= 256 {
            return Err(HostError::LimitExceeded {
                resource: "standalone pending header preloads".into(),
            });
        }
        queue
            .borrow_mut()
            .jobs
            .try_reserve(1)
            .map_err(|_| HostError::LimitExceeded {
                resource: "standalone preload header queue".into(),
            })?;
        let source = begin_local_preload(scope, &backend, key)?;
        let mut queue = queue.borrow_mut();
        queue.owners.insert(
            environment.id,
            (Rc::downgrade(state), state.document_authority_epoch()),
        );
        queue.headers.insert((environment.id, identity.clone()));
        queue.jobs.push_back(GeneralPreload {
            preparation_pauses: Default::default(),
            owner: state.clone(),
            environment,
            epoch: state.document_authority_epoch(),
            cancellation: state.document_load_cancellation_epoch(),
            target: None,
            key: identity,
            source: GeneralPreloadSource::Local(source),
            publication: None,
            result: None,
            first_error: None,
            retry: Instant::now(),
            deadline: Instant::now() + Duration::from_secs(70),
        });
    }
    Ok(())
}
