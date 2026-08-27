use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use oryn_common::v2::{
    ActionId, ActionResult, CONTRACT_VERSION, CapabilityDiagnostic, Effect, ExecutionDomain,
    Observation, ObservationDelta, ObservationSource, PageInfo, ProjectionProfile, Revision,
    SemanticAction, SemanticRef, SupportLevel,
};
use serde::{Deserialize, Serialize};
#[cfg(feature = "v8-host")]
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, oneshot};
use url::Url;

use crate::html::{ParsedDocument, parse};
use crate::semantic::{
    empty_observation, is_hidden, project_document, project_document_full, project_element,
};
use crate::trace::{TraceEvent, TraceEventKind, TraceLog};
use crate::{
    network::{NetworkBroker, NetworkRequest, PolicyBroker, RustlsTransport, SharedNetworkBroker},
    security::NetworkPolicy,
};

static NEXT_CONTEXT_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_PAGE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone)]
pub struct Browser {
    config: BrowserConfig,
}

#[derive(Clone)]
pub struct BrowserContext {
    id: u64,
    config: BrowserConfig,
    broker: Option<SharedNetworkBroker>,
}

#[derive(Debug, Clone)]
pub struct PageHandle {
    id: u64,
    sender: mpsc::Sender<PageCommand>,
}

#[derive(Debug, Clone)]
pub struct BrowserConfig {
    pub worker_queue_capacity: usize,
    pub request_timeout: Duration,
    pub max_response_bytes: usize,
    pub max_redirects: usize,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            worker_queue_capacity: 32,
            request_timeout: Duration::from_secs(30),
            max_response_bytes: 16 * 1024 * 1024,
            max_redirects: 10,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ContextOptions {
    /// Context-owned cookies and storage are discarded when the context closes.
    pub ephemeral: bool,
    pub network_policy: NetworkPolicy,
}

impl Default for ContextOptions {
    fn default() -> Self {
        Self {
            ephemeral: true,
            network_policy: NetworkPolicy::default(),
        }
    }
}

impl ContextOptions {
    pub fn loopback_test() -> Self {
        let network_policy = NetworkPolicy {
            allow_loopback: true,
            ..NetworkPolicy::default()
        };
        Self {
            ephemeral: true,
            network_policy,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleState {
    Commit,
    DomContentLoaded,
    Load,
    NetworkIdle,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NavigationResult {
    pub requested_url: String,
    pub final_url: String,
    pub status: u16,
    pub revision: Revision,
    pub lifecycle: Vec<LifecycleState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitPredicate {
    Lifecycle(LifecycleState),
    UrlContains(String),
    SelectorExists(String),
    SelectorGone(String),
    SelectorVisible(String),
    SelectorHidden(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentKind {
    Html,
    Text,
    Links,
    Images,
    Tables,
}

enum PageCommand {
    LoadHtml {
        url: String,
        html: String,
        reply: oneshot::Sender<Result<Revision, RuntimeError>>,
    },
    Goto {
        url: String,
        reply: oneshot::Sender<Result<NavigationResult, RuntimeError>>,
    },
    Back {
        reply: oneshot::Sender<Result<NavigationResult, RuntimeError>>,
    },
    Forward {
        reply: oneshot::Sender<Result<NavigationResult, RuntimeError>>,
    },
    Refresh {
        reply: oneshot::Sender<Result<NavigationResult, RuntimeError>>,
    },
    WaitFor {
        predicate: WaitPredicate,
        reply: oneshot::Sender<Result<(), RuntimeError>>,
    },
    Evaluate {
        source: String,
        reply: oneshot::Sender<Result<String, RuntimeError>>,
    },
    Capabilities {
        reply: oneshot::Sender<Vec<CapabilityDiagnostic>>,
    },
    ResolveSelector {
        selector: String,
        action: SemanticAction,
        reply: oneshot::Sender<Result<SemanticRef, RuntimeError>>,
    },
    Extract {
        kind: ContentKind,
        selector: Option<String>,
        reply: oneshot::Sender<Result<serde_json::Value, RuntimeError>>,
    },
    Observe {
        profile: ProjectionProfile,
        reply: oneshot::Sender<Observation>,
    },
    ObserveDelta {
        from: Revision,
        reply: oneshot::Sender<Result<ObservationDelta, RuntimeError>>,
    },
    Trace {
        reply: oneshot::Sender<Vec<TraceEvent>>,
    },
    Execute {
        action: NativeAction,
        reply: oneshot::Sender<Result<ActionResult, RuntimeError>>,
    },
    Close,
}

impl Browser {
    pub fn new() -> Self {
        Self::with_config(BrowserConfig::default())
    }

    pub fn with_config(config: BrowserConfig) -> Self {
        Self { config }
    }

    pub fn new_context(&self) -> BrowserContext {
        self.new_context_with_options(ContextOptions::default())
    }

    pub fn new_context_with_options(&self, options: ContextOptions) -> BrowserContext {
        let timeout = self.config.request_timeout;
        let max_response_bytes = self.config.max_response_bytes;
        let network_policy = options.network_policy.clone();
        let broker = std::thread::spawn(move || {
            RustlsTransport::new(timeout, max_response_bytes)
                .ok()
                .map(|transport| {
                    SharedNetworkBroker::new(PolicyBroker::new(network_policy, transport))
                })
        })
        .join()
        .unwrap_or(None);
        BrowserContext {
            id: NEXT_CONTEXT_ID.fetch_add(1, Ordering::Relaxed),
            config: self.config.clone(),
            broker,
        }
    }
}

impl Default for Browser {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserContext {
    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn new_page(&self) -> PageHandle {
        let (sender, mut receiver) = mpsc::channel(self.config.worker_queue_capacity);
        let id = NEXT_PAGE_ID.fetch_add(1, Ordering::Relaxed);
        let config = self.config.clone();
        let broker = self.broker.clone();
        std::thread::spawn(move || {
            let mut state = PageState::new(config, broker);
            while let Some(command) = receiver.blocking_recv() {
                match command {
                    PageCommand::LoadHtml { url, html, reply } => {
                        state.trace.push(
                            Revision(state.revision),
                            None,
                            TraceEventKind::NavigationStarted { url: url.clone() },
                        );
                        if let Err(error) =
                            state.load_document(url.clone(), &html, true, &BTreeMap::new())
                        {
                            let _ = reply.send(Err(error));
                            continue;
                        }
                        let _ = reply.send(Ok(Revision(state.revision)));
                    }
                    PageCommand::Goto { url, reply } => {
                        let _ = reply.send(state.navigate(url, HistoryUpdate::Push));
                    }
                    PageCommand::Back { reply } => {
                        let _ = reply.send(state.traverse_history(-1));
                    }
                    PageCommand::Forward { reply } => {
                        let _ = reply.send(state.traverse_history(1));
                    }
                    PageCommand::Refresh { reply } => {
                        let _ = reply.send(state.refresh());
                    }
                    PageCommand::WaitFor { predicate, reply } => {
                        let _ = reply.send(state.wait_for(&predicate));
                    }
                    PageCommand::Evaluate { source, reply } => {
                        let _ = reply.send(state.evaluate(&source));
                    }
                    PageCommand::Capabilities { reply } => {
                        let _ = reply.send(state.capabilities());
                    }
                    PageCommand::ResolveSelector {
                        selector,
                        action,
                        reply,
                    } => {
                        let _ = reply.send(state.resolve_selector(&selector, action));
                    }
                    PageCommand::Extract {
                        kind,
                        selector,
                        reply,
                    } => {
                        let _ = reply.send(state.extract(kind, selector.as_deref()));
                    }
                    PageCommand::Observe { profile, reply } => {
                        let _ = reply.send(state.observation(profile));
                    }
                    PageCommand::ObserveDelta { from, reply } => {
                        let _ = reply.send(state.delta_from(from));
                    }
                    PageCommand::Trace { reply } => {
                        let _ = reply.send(state.trace.events().to_vec());
                    }
                    PageCommand::Execute { action, reply } => {
                        let _ = reply.send(state.execute(action));
                    }
                    PageCommand::Close => break,
                }
            }
        });
        PageHandle { id, sender }
    }
}

impl PageHandle {
    pub fn id(&self) -> u64 {
        self.id
    }

    pub async fn load_html(
        &self,
        url: impl Into<String>,
        html: impl Into<String>,
    ) -> Result<Revision, RuntimeError> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(PageCommand::LoadHtml {
                url: url.into(),
                html: html.into(),
                reply,
            })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        response.await.map_err(|_| RuntimeError::Closed)?
    }

    pub async fn goto(&self, url: impl Into<String>) -> Result<NavigationResult, RuntimeError> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(PageCommand::Goto {
                url: url.into(),
                reply,
            })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        response.await.map_err(|_| RuntimeError::Closed)?
    }

    pub async fn back(&self) -> Result<NavigationResult, RuntimeError> {
        self.navigation_command(|reply| PageCommand::Back { reply })
            .await
    }

    pub async fn forward(&self) -> Result<NavigationResult, RuntimeError> {
        self.navigation_command(|reply| PageCommand::Forward { reply })
            .await
    }

    pub async fn refresh(&self) -> Result<NavigationResult, RuntimeError> {
        self.navigation_command(|reply| PageCommand::Refresh { reply })
            .await
    }

    async fn navigation_command(
        &self,
        command: impl FnOnce(oneshot::Sender<Result<NavigationResult, RuntimeError>>) -> PageCommand,
    ) -> Result<NavigationResult, RuntimeError> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(command(reply))
            .await
            .map_err(|_| RuntimeError::Closed)?;
        response.await.map_err(|_| RuntimeError::Closed)?
    }

    pub async fn wait_for(&self, predicate: WaitPredicate) -> Result<(), RuntimeError> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(PageCommand::WaitFor { predicate, reply })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        response.await.map_err(|_| RuntimeError::Closed)?
    }

    pub async fn evaluate(&self, source: impl Into<String>) -> Result<String, RuntimeError> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(PageCommand::Evaluate {
                source: source.into(),
                reply,
            })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        response.await.map_err(|_| RuntimeError::Closed)?
    }

    pub async fn capabilities(&self) -> Result<Vec<CapabilityDiagnostic>, RuntimeError> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(PageCommand::Capabilities { reply })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        response.await.map_err(|_| RuntimeError::Closed)
    }

    pub async fn resolve_selector(
        &self,
        selector: impl Into<String>,
        action: SemanticAction,
    ) -> Result<SemanticRef, RuntimeError> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(PageCommand::ResolveSelector {
                selector: selector.into(),
                action,
                reply,
            })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        response.await.map_err(|_| RuntimeError::Closed)?
    }

    pub async fn extract(
        &self,
        kind: ContentKind,
        selector: Option<String>,
    ) -> Result<serde_json::Value, RuntimeError> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(PageCommand::Extract {
                kind,
                selector,
                reply,
            })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        response.await.map_err(|_| RuntimeError::Closed)?
    }

    pub async fn observe(&self) -> Result<Observation, RuntimeError> {
        self.observe_profile(ProjectionProfile::Compact).await
    }

    pub async fn observe_profile(
        &self,
        profile: ProjectionProfile,
    ) -> Result<Observation, RuntimeError> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(PageCommand::Observe { profile, reply })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        response.await.map_err(|_| RuntimeError::Closed)
    }

    pub async fn observe_delta(&self, from: Revision) -> Result<ObservationDelta, RuntimeError> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(PageCommand::ObserveDelta { from, reply })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        response.await.map_err(|_| RuntimeError::Closed)?
    }

    pub async fn trace(&self) -> Result<Vec<TraceEvent>, RuntimeError> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(PageCommand::Trace { reply })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        response.await.map_err(|_| RuntimeError::Closed)
    }

    pub async fn execute(&self, action: NativeAction) -> Result<ActionResult, RuntimeError> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(PageCommand::Execute { action, reply })
            .await
            .map_err(|_| RuntimeError::Closed)?;
        response.await.map_err(|_| RuntimeError::Closed)?
    }

    pub async fn close(&self) -> Result<(), RuntimeError> {
        self.sender
            .send(PageCommand::Close)
            .await
            .map_err(|_| RuntimeError::Closed)
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum RuntimeError {
    #[error("page runtime is closed")]
    Closed,
    #[error("semantic reference belongs to document generation {actual}, expected {expected}")]
    WrongDocumentGeneration { expected: u64, actual: u64 },
    #[error("semantic reference is stale or unknown")]
    StaleSemanticRef,
    #[error("{action:?} is not supported by this element")]
    UnsupportedAction { action: SemanticAction },
    #[error("semantic target is disabled")]
    Disabled,
    #[error("revision {0:?} is not available for delta observation")]
    UnknownRevision(Revision),
    #[error("native script realm failed: {0}")]
    Script(String),
    #[error("network navigation failed: {0}")]
    Network(String),
    #[error("navigation history has no entry in that direction")]
    HistoryBoundary,
    #[error("wait predicate is not currently satisfied: {0}")]
    WaitUnsatisfied(String),
    #[error("JavaScript evaluation requires the v8-host feature")]
    EvaluationUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeAction {
    pub target: SemanticRef,
    pub action: SemanticAction,
    pub value: Option<String>,
}

struct PageState {
    config: BrowserConfig,
    url: String,
    document_generation: u64,
    revision: u64,
    next_action_id: u64,
    document: Option<ParsedDocument>,
    #[cfg(feature = "v8-host")]
    executable: Option<crate::executable::ExecutableDocument>,
    snapshots: BTreeMap<u64, Vec<oryn_common::v2::SemanticNodeView>>,
    trace: TraceLog,
    history: Vec<String>,
    history_index: Option<usize>,
    broker: Option<SharedNetworkBroker>,
    #[cfg(feature = "v8-host")]
    resource_cache: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy)]
enum HistoryUpdate {
    Push,
    Preserve,
}

impl PageState {
    fn new(config: BrowserConfig, broker: Option<SharedNetworkBroker>) -> Self {
        Self {
            config,
            url: String::new(),
            document_generation: 0,
            revision: 0,
            next_action_id: 0,
            document: None,
            #[cfg(feature = "v8-host")]
            executable: None,
            snapshots: BTreeMap::new(),
            trace: TraceLog::default(),
            history: Vec::new(),
            history_index: None,
            broker,
            #[cfg(feature = "v8-host")]
            resource_cache: BTreeMap::new(),
        }
    }

    fn load_document(
        &mut self,
        url: String,
        html: &str,
        push_history: bool,
        resources: &BTreeMap<String, String>,
    ) -> Result<(), RuntimeError> {
        #[cfg(not(feature = "v8-host"))]
        let _ = resources;
        self.document_generation += 1;
        self.revision += 1;
        self.url = url;
        #[cfg(feature = "v8-host")]
        {
            // V8 enters isolates on creation and requires reverse-order drops;
            // retire the previous document realm before creating its successor.
            let storage = self
                .executable
                .as_mut()
                .and_then(|executable| executable.storage_snapshot().ok())
                .unwrap_or_else(|| ("[]".into(), "[]".into()));
            self.executable = None;
            let mut resources = resources.clone();
            resources.insert("oryn:local-storage".into(), storage.0);
            resources.insert("oryn:session-storage".into(), storage.1);
            let executable = crate::executable::ExecutableDocument::load_with_environment(
                html,
                &resources,
                self.broker.clone(),
                Url::parse(&self.url).ok(),
            )
            .map_err(|error| RuntimeError::Script(error.to_string()))?;
            for diagnostic in &executable.diagnostics {
                self.trace.push(
                    Revision(self.revision),
                    None,
                    TraceEventKind::ScriptDiagnostic {
                        capability: diagnostic.capability.clone(),
                        detail: diagnostic.detail.clone().unwrap_or_default(),
                    },
                );
            }
            self.document = Some(executable.document.clone());
            self.executable = Some(executable);
            if let Some(executable) = self.executable.as_mut()
                && let Ok(entries) = executable.take_console_entries()
            {
                for entry in entries {
                    self.trace.push(
                        Revision(self.revision),
                        None,
                        TraceEventKind::Console {
                            level: entry.level,
                            message: entry.message,
                        },
                    );
                }
            }
        }
        #[cfg(not(feature = "v8-host"))]
        {
            self.document = Some(parse(html));
        }
        if push_history {
            if let Some(index) = self.history_index {
                self.history.truncate(index + 1);
            }
            self.history.push(self.url.clone());
            self.history_index = Some(self.history.len() - 1);
        }
        self.record_snapshot();
        self.trace.push(
            Revision(self.revision),
            None,
            TraceEventKind::NavigationCommitted {
                url: self.url.clone(),
            },
        );
        for state in [
            LifecycleState::DomContentLoaded,
            LifecycleState::Load,
            LifecycleState::NetworkIdle,
        ] {
            self.trace.push(
                Revision(self.revision),
                None,
                TraceEventKind::LifecycleChanged { state },
            );
        }
        Ok(())
    }

    fn navigate(
        &mut self,
        requested_url: String,
        history_update: HistoryUpdate,
    ) -> Result<NavigationResult, RuntimeError> {
        let mut current = Url::parse(&requested_url)
            .map_err(|error| RuntimeError::Network(format!("invalid URL: {error}")))?;
        let mut redirects = 0_usize;
        self.trace.push(
            Revision(self.revision),
            None,
            TraceEventKind::NavigationStarted {
                url: requested_url.clone(),
            },
        );
        let (status, body) = loop {
            self.trace.push(
                Revision(self.revision),
                None,
                TraceEventKind::RequestStarted {
                    method: "GET".into(),
                    url: current.to_string(),
                },
            );
            let broker = self.broker.as_ref().ok_or_else(|| {
                RuntimeError::Network("network transport initialization failed".into())
            })?;
            let response = broker
                .lock()
                .map_err(|_| RuntimeError::Network("network broker lock poisoned".into()))?
                .send(NetworkRequest {
                    request_id: self.revision + redirects as u64 + 1,
                    method: "GET".into(),
                    url: current.to_string(),
                    headers: vec![(
                        "accept".into(),
                        "text/html,application/xhtml+xml;q=0.9,*/*;q=0.8".into(),
                    )],
                    body: Vec::new(),
                })
                .map_err(|error| {
                    self.trace.push(
                        Revision(self.revision),
                        None,
                        TraceEventKind::PolicyDenied {
                            capability: format!("network.navigation: {error}"),
                        },
                    );
                    RuntimeError::Network(error.to_string())
                })?;
            self.trace.push(
                Revision(self.revision),
                None,
                TraceEventKind::ResponseReceived {
                    url: current.to_string(),
                    status: response.status,
                    bytes: response.body.len(),
                },
            );
            if (300..400).contains(&response.status) {
                let location = response
                    .headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("location"))
                    .map(|(_, value)| value)
                    .ok_or_else(|| RuntimeError::Network("redirect omitted Location".into()))?;
                redirects += 1;
                if redirects > self.config.max_redirects {
                    return Err(RuntimeError::Network(format!(
                        "redirect limit ({}) exceeded",
                        self.config.max_redirects
                    )));
                }
                current = current
                    .join(location)
                    .map_err(|error| RuntimeError::Network(format!("invalid redirect: {error}")))?;
                continue;
            }
            break (response.status, response.body);
        };
        let html = String::from_utf8_lossy(&body).into_owned();
        let html = self.inline_external_stylesheets(&current, &html)?;
        let resources = self.load_external_scripts(&current, &html)?;
        self.load_document(
            current.to_string(),
            &html,
            matches!(history_update, HistoryUpdate::Push),
            &resources,
        )?;
        Ok(NavigationResult {
            requested_url,
            final_url: self.url.clone(),
            status,
            revision: Revision(self.revision),
            lifecycle: vec![
                LifecycleState::Commit,
                LifecycleState::DomContentLoaded,
                LifecycleState::Load,
                LifecycleState::NetworkIdle,
            ],
        })
    }

    #[cfg(feature = "v8-host")]
    fn inline_external_stylesheets(
        &mut self,
        base_url: &Url,
        html: &str,
    ) -> Result<String, RuntimeError> {
        let parsed = parse(html);
        let hrefs = parsed
            .dom
            .iter()
            .filter_map(|(_, node)| match &node.kind {
                crate::dom::NodeKind::Element { name }
                    if name.local.as_ref() == "link"
                        && node.attributes.get("rel").is_some_and(|value| {
                            value
                                .split_whitespace()
                                .any(|token| token.eq_ignore_ascii_case("stylesheet"))
                        }) =>
                {
                    node.attributes.get("href").cloned()
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut styles = String::new();
        for href in hrefs {
            let absolute = base_url.join(&href).map_err(|error| {
                RuntimeError::Network(format!("invalid stylesheet URL {href}: {error}"))
            })?;
            let cache_key = absolute.to_string();
            let css = if let Some(cached) = self.resource_cache.get(&cache_key) {
                cached.clone()
            } else {
                let css =
                    self.fetch_external_resource(absolute, "text/css,*/*;q=0.1", "stylesheet")?;
                self.resource_cache.insert(cache_key, css.clone());
                css
            };
            styles.push_str("\n<style data-oryn-external=\"");
            styles.push_str(&href.replace('&', "&amp;").replace('"', "&quot;"));
            styles.push_str("\">\n");
            styles.push_str(&css);
            styles.push_str("\n</style>");
        }
        let mut expanded = html.to_string();
        let head_end = expanded.to_ascii_lowercase().rfind("</head>");
        if let Some(index) = head_end {
            expanded.insert_str(index, &styles);
        } else {
            expanded.push_str(&styles);
        }
        Ok(expanded)
    }

    #[cfg(not(feature = "v8-host"))]
    fn inline_external_stylesheets(
        &mut self,
        _base_url: &Url,
        html: &str,
    ) -> Result<String, RuntimeError> {
        Ok(html.to_string())
    }

    #[cfg(feature = "v8-host")]
    fn load_external_scripts(
        &mut self,
        base_url: &Url,
        html: &str,
    ) -> Result<BTreeMap<String, String>, RuntimeError> {
        let parsed = parse(html);
        let sources = parsed
            .dom
            .iter()
            .filter_map(|(_, node)| match &node.kind {
                crate::dom::NodeKind::Element { name }
                    if matches!(name.local.as_ref(), "script" | "iframe") =>
                {
                    node.attributes
                        .get("src")
                        .cloned()
                        .map(|source| (source, name.local.as_ref() == "iframe"))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut resources = BTreeMap::new();
        let mut pending = sources
            .into_iter()
            .map(|(source, frame)| (base_url.clone(), source, frame))
            .collect::<Vec<_>>();
        while let Some((parent, source, frame)) = pending.pop() {
            if resources.contains_key(&source) {
                continue;
            }
            let absolute = parent.join(&source).map_err(|error| {
                RuntimeError::Network(format!("invalid external script URL {source}: {error}"))
            })?;
            if frame && origin_key(&absolute) != origin_key(base_url) {
                continue;
            }
            let cache_key = absolute.to_string();
            let (request_url, expected_digest) = pinned_resource(&absolute)
                .map(|(url, digest)| (Url::parse(url).expect("valid pinned URL"), Some(digest)))
                .unwrap_or_else(|| (absolute.clone(), None));
            let body = if let Some(cached) = self.resource_cache.get(&cache_key) {
                cached.clone()
            } else {
                let body = self.fetch_external_resource(
                    request_url,
                    "text/javascript,*/*;q=0.1",
                    "script",
                )?;
                if let Some(expected) = expected_digest {
                    let actual = format!("{:x}", Sha256::digest(body.as_bytes()));
                    if actual != expected {
                        return Err(RuntimeError::Network(format!(
                            "pinned resource integrity failure for {absolute}: expected {expected}, got {actual}"
                        )));
                    }
                }
                self.resource_cache.insert(cache_key, body.clone());
                body
            };
            for dependency in module_specifiers(&body) {
                if !resources.contains_key(&dependency) {
                    pending.push((absolute.clone(), dependency, false));
                }
            }
            resources.insert(source, body);
        }
        Ok(resources)
    }

    #[cfg(not(feature = "v8-host"))]
    fn load_external_scripts(
        &mut self,
        _base_url: &Url,
        _html: &str,
    ) -> Result<BTreeMap<String, String>, RuntimeError> {
        Ok(BTreeMap::new())
    }

    #[cfg(feature = "v8-host")]
    fn fetch_external_resource(
        &mut self,
        mut url: Url,
        accept: &str,
        resource_kind: &str,
    ) -> Result<String, RuntimeError> {
        for redirect in 0..=self.config.max_redirects {
            self.trace.push(
                Revision(self.revision),
                None,
                TraceEventKind::RequestStarted {
                    method: "GET".into(),
                    url: url.to_string(),
                },
            );
            let response = self
                .broker
                .as_ref()
                .ok_or_else(|| {
                    RuntimeError::Network("network transport initialization failed".into())
                })?
                .lock()
                .map_err(|_| RuntimeError::Network("network broker lock poisoned".into()))?
                .send(NetworkRequest {
                    request_id: self.revision + redirect as u64 + 10_000,
                    method: "GET".into(),
                    url: url.to_string(),
                    headers: vec![("accept".into(), accept.into())],
                    body: Vec::new(),
                })
                .map_err(|error| RuntimeError::Network(error.to_string()))?;
            self.trace.push(
                Revision(self.revision),
                None,
                TraceEventKind::ResponseReceived {
                    url: url.to_string(),
                    status: response.status,
                    bytes: response.body.len(),
                },
            );
            if (300..400).contains(&response.status) {
                let location = response
                    .headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("location"))
                    .map(|(_, value)| value)
                    .ok_or_else(|| {
                        RuntimeError::Network(format!("{resource_kind} redirect omitted Location"))
                    })?;
                url = url.join(location).map_err(|error| {
                    RuntimeError::Network(format!("invalid {resource_kind} redirect: {error}"))
                })?;
                continue;
            }
            if !(200..300).contains(&response.status) {
                return Err(RuntimeError::Network(format!(
                    "external {resource_kind} returned HTTP {}",
                    response.status,
                )));
            }
            return Ok(String::from_utf8_lossy(&response.body).into_owned());
        }
        Err(RuntimeError::Network(format!(
            "{resource_kind} redirect limit ({}) exceeded",
            self.config.max_redirects,
        )))
    }

    fn traverse_history(&mut self, delta: isize) -> Result<NavigationResult, RuntimeError> {
        let current = self.history_index.ok_or(RuntimeError::HistoryBoundary)? as isize;
        let next = current + delta;
        if next < 0 || next >= self.history.len() as isize {
            return Err(RuntimeError::HistoryBoundary);
        }
        let requested = self.history[next as usize].clone();
        let result = self.navigate(requested, HistoryUpdate::Preserve)?;
        self.history_index = Some(next as usize);
        Ok(result)
    }

    fn refresh(&mut self) -> Result<NavigationResult, RuntimeError> {
        if self.url.is_empty() {
            return Err(RuntimeError::HistoryBoundary);
        }
        self.navigate(self.url.clone(), HistoryUpdate::Preserve)
    }

    fn wait_for(&mut self, predicate: &WaitPredicate) -> Result<(), RuntimeError> {
        if self.wait_satisfied(predicate) {
            return Ok(());
        }
        #[cfg(feature = "v8-host")]
        if let Some(executable) = self.executable.as_mut() {
            executable
                .drain_tasks()
                .map_err(|error| RuntimeError::Script(error.to_string()))?;
            self.document = Some(executable.document.clone());
        }
        self.wait_satisfied(predicate)
            .then_some(())
            .ok_or_else(|| RuntimeError::WaitUnsatisfied(format!("{predicate:?}")))
    }

    fn wait_satisfied(&self, predicate: &WaitPredicate) -> bool {
        match predicate {
            WaitPredicate::Lifecycle(_) => self.document.is_some(),
            WaitPredicate::UrlContains(fragment) => self.url.contains(fragment),
            WaitPredicate::SelectorExists(selector) => self
                .document
                .as_ref()
                .is_some_and(|document| selector_nodes(document, selector).next().is_some()),
            WaitPredicate::SelectorGone(selector) => !self
                .document
                .as_ref()
                .is_some_and(|document| selector_nodes(document, selector).next().is_some()),
            WaitPredicate::SelectorVisible(selector) => {
                self.document.as_ref().is_some_and(|document| {
                    selector_nodes(document, selector).any(|id| {
                        project_element(&document.dom, id, self.document_generation, id.slot + 1)
                            .is_some_and(|node| {
                                node.states.contains(&oryn_common::v2::NodeState::Visible)
                            })
                    })
                })
            }
            WaitPredicate::SelectorHidden(selector) => {
                self.document.as_ref().is_some_and(|document| {
                    let mut nodes = selector_nodes(document, selector).peekable();
                    nodes.peek().is_none()
                        || nodes.all(|id| {
                            project_element(
                                &document.dom,
                                id,
                                self.document_generation,
                                id.slot + 1,
                            )
                            .is_some_and(|node| {
                                node.states.contains(&oryn_common::v2::NodeState::Hidden)
                            })
                        })
                })
            }
        }
    }

    fn evaluate(&mut self, source: &str) -> Result<String, RuntimeError> {
        #[cfg(feature = "v8-host")]
        {
            let executable = self
                .executable
                .as_mut()
                .ok_or(RuntimeError::EvaluationUnavailable)?;
            let value = executable
                .evaluate(source)
                .map_err(|error| RuntimeError::Script(error.to_string()))?;
            self.document = Some(executable.document.clone());
            Ok(value)
        }
        #[cfg(not(feature = "v8-host"))]
        {
            let _ = source;
            Err(RuntimeError::EvaluationUnavailable)
        }
    }

    fn capabilities(&self) -> Vec<CapabilityDiagnostic> {
        let layout = CapabilityDiagnostic {
            capability: "rendered_layout_and_paint".into(),
            support: SupportLevel::Unsupported,
            alternatives: vec![ExecutionDomain::Chromium, ExecutionDomain::Webkit],
            handoff_lossy: true,
            detail: Some(
                "native Oryn exposes deterministic document-order geometry, not rendered layout or paint"
                    .into(),
            ),
        };
        #[cfg(feature = "v8-host")]
        {
            let mut capabilities = self
                .executable
                .as_ref()
                .map(|executable| executable.diagnostics.clone())
                .unwrap_or_default();
            capabilities.push(layout);
            capabilities
        }
        #[cfg(not(feature = "v8-host"))]
        {
            vec![
                CapabilityDiagnostic {
                    capability: "javascript.execution".into(),
                    support: SupportLevel::Unsupported,
                    alternatives: vec![
                        ExecutionDomain::Chromium,
                        ExecutionDomain::Webkit,
                        ExecutionDomain::UserBrowser,
                    ],
                    handoff_lossy: true,
                    detail: Some(
                        "this binary was built without the v8-host feature and is a probe, not the production native runtime"
                            .into(),
                    ),
                },
                layout,
            ]
        }
    }

    fn resolve_selector(
        &self,
        selector: &str,
        action: SemanticAction,
    ) -> Result<SemanticRef, RuntimeError> {
        let document = self
            .document
            .as_ref()
            .ok_or(RuntimeError::StaleSemanticRef)?;
        selector_nodes(document, selector)
            .filter_map(|id| {
                project_element(&document.dom, id, self.document_generation, id.slot + 1)
            })
            .find(|node| node.actions.contains(&action))
            .map(|node| node.semantic_ref)
            .ok_or(RuntimeError::StaleSemanticRef)
    }

    fn extract(
        &self,
        kind: ContentKind,
        selector: Option<&str>,
    ) -> Result<serde_json::Value, RuntimeError> {
        let document = self
            .document
            .as_ref()
            .ok_or(RuntimeError::StaleSemanticRef)?;
        let roots = selector
            .map(|selector| selector_nodes(document, selector).collect::<Vec<_>>())
            .unwrap_or_else(|| vec![document.document]);
        match kind {
            ContentKind::Html => Ok(serde_json::Value::String(
                roots
                    .iter()
                    .map(|id| serialize_dom_node(&document.dom, *id))
                    .collect::<Vec<_>>()
                    .join(""),
            )),
            ContentKind::Text => Ok(serde_json::Value::String(
                roots
                    .iter()
                    .filter_map(|id| visible_text_content(&document.dom, *id).ok())
                    .collect::<Vec<_>>()
                    .join(" ")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
            )),
            ContentKind::Links => Ok(serde_json::Value::Array(
                document
                    .dom
                    .iter()
                    .filter(|(_, node)| {
                        matches!(&node.kind, crate::dom::NodeKind::Element { name } if name.local.as_ref() == "a")
                    })
                    .map(|(id, node)| {
                        serde_json::json!({
                            "text": document.dom.text_content(id).unwrap_or_default(),
                            "href": node.attributes.get("href").cloned().unwrap_or_default()
                        })
                    })
                    .collect(),
            )),
            ContentKind::Images => Ok(serde_json::Value::Array(
                document
                    .dom
                    .iter()
                    .filter(|(_, node)| {
                        matches!(&node.kind, crate::dom::NodeKind::Element { name } if name.local.as_ref() == "img")
                    })
                    .map(|(_, node)| {
                        serde_json::json!({
                            "src": node.attributes.get("src").cloned().unwrap_or_default(),
                            "alt": node.attributes.get("alt").cloned().unwrap_or_default()
                        })
                    })
                    .collect(),
            )),
            ContentKind::Tables => Ok(serde_json::Value::Array(
                document
                    .dom
                    .iter()
                    .filter(|(_, node)| {
                        matches!(&node.kind, crate::dom::NodeKind::Element { name } if name.local.as_ref() == "table")
                    })
                    .map(|(table, _)| {
                        serde_json::Value::Array(
                            descendants_of(&document.dom, table)
                                .into_iter()
                                .filter(|id| {
                                    document.dom.get(*id).is_ok_and(|node| {
                                        matches!(&node.kind, crate::dom::NodeKind::Element { name } if name.local.as_ref() == "tr")
                                    })
                                })
                                .map(|row| {
                                    serde_json::Value::Array(
                                        document
                                            .dom
                                            .get(row)
                                            .map(|node| {
                                                node.children
                                                    .iter()
                                                    .filter_map(|cell| {
                                                        document.dom.get(*cell).ok().and_then(|node| {
                                                            matches!(&node.kind, crate::dom::NodeKind::Element { name } if matches!(name.local.as_ref(), "td" | "th"))
                                                                .then(|| serde_json::Value::String(document.dom.text_content(*cell).unwrap_or_default()))
                                                        })
                                                    })
                                                    .collect()
                                            })
                                            .unwrap_or_default(),
                                    )
                                })
                                .collect(),
                        )
                    })
                    .collect(),
            )),
        }
    }

    fn observation(&self, profile: ProjectionProfile) -> Observation {
        let Some(document) = &self.document else {
            let mut observation = empty_observation("about:blank".into(), String::new(), 0);
            observation.profile = profile;
            return observation;
        };
        Observation {
            contract_version: CONTRACT_VERSION,
            page: PageInfo {
                url: self.url.clone(),
                title: document_title(document),
                document_generation: self.document_generation,
            },
            revision: Revision(self.revision),
            profile,
            source: ObservationSource::NativeDom,
            nodes: match profile {
                ProjectionProfile::Full => {
                    project_document_full(&document.dom, self.document_generation)
                }
                _ => project_document(&document.dom, self.document_generation),
            },
            capabilities: self.capabilities(),
            diagnostics: document.errors.clone(),
            execution_domain: ExecutionDomain::Native,
        }
    }

    fn record_snapshot(&mut self) {
        let nodes = self
            .document
            .as_ref()
            .map(|document| project_document(&document.dom, self.document_generation))
            .unwrap_or_default();
        self.snapshots.insert(self.revision, nodes);
    }

    fn delta_from(&self, from: Revision) -> Result<ObservationDelta, RuntimeError> {
        let old = if from.0 == 0 {
            &[][..]
        } else {
            self.snapshots
                .get(&from.0)
                .map(Vec::as_slice)
                .ok_or(RuntimeError::UnknownRevision(from))?
        };
        let current = self
            .snapshots
            .get(&self.revision)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let old_by_ref: BTreeMap<_, _> = old.iter().map(|node| (node.semantic_ref, node)).collect();
        let current_by_ref: BTreeMap<_, _> = current
            .iter()
            .map(|node| (node.semantic_ref, node))
            .collect();
        Ok(ObservationDelta {
            contract_version: CONTRACT_VERSION,
            from_revision: from,
            to_revision: Revision(self.revision),
            upserted: current
                .iter()
                .filter(|node| old_by_ref.get(&node.semantic_ref).copied() != Some(*node))
                .cloned()
                .collect(),
            removed: old_by_ref
                .keys()
                .filter(|semantic_ref| !current_by_ref.contains_key(semantic_ref))
                .copied()
                .collect(),
        })
    }

    fn execute(&mut self, action: NativeAction) -> Result<ActionResult, RuntimeError> {
        if action.target.document_generation != self.document_generation {
            return Err(RuntimeError::WrongDocumentGeneration {
                expected: self.document_generation,
                actual: action.target.document_generation,
            });
        }
        #[cfg(feature = "v8-host")]
        if self.executable.is_some() {
            return self.execute_in_realm(action);
        }
        let document = self
            .document
            .as_mut()
            .ok_or(RuntimeError::StaleSemanticRef)?;
        let node_id = crate::semantic::unpack_node_ref(action.target.node);
        let node = document
            .dom
            .get_mut(node_id)
            .map_err(|_| RuntimeError::StaleSemanticRef)?;
        let crate::dom::NodeKind::Element { name } = &node.kind else {
            return Err(RuntimeError::StaleSemanticRef);
        };
        if node.attributes.contains_key("disabled") {
            return Err(RuntimeError::Disabled);
        }
        let local_name = name.local.as_ref();
        let (summary, mutated) = match action.action {
            SemanticAction::Type if matches!(local_name, "input" | "textarea") => {
                let value = action.value.unwrap_or_default();
                node.attributes.insert("value".into(), value);
                ("updated form control value", true)
            }
            SemanticAction::Clear if matches!(local_name, "input" | "textarea") => {
                node.attributes.insert("value".into(), String::new());
                ("cleared form control value", true)
            }
            SemanticAction::Click if matches!(local_name, "button" | "a" | "input" | "select") => {
                let toggled = local_name == "input"
                    && matches!(
                        node.attributes.get("type").map(String::as_str),
                        Some("checkbox" | "radio")
                    );
                if toggled && node.attributes.remove("checked").is_none() {
                    node.attributes.insert("checked".into(), String::new());
                }
                ("dispatched click activation", toggled)
            }
            SemanticAction::Check if local_name == "input" => {
                node.attributes.insert("checked".into(), String::new());
                ("checked form control", true)
            }
            SemanticAction::Uncheck if local_name == "input" => {
                node.attributes.remove("checked");
                ("unchecked form control", true)
            }
            SemanticAction::Select if local_name == "select" => {
                node.attributes
                    .insert("value".into(), action.value.unwrap_or_default());
                ("selected form control value", true)
            }
            SemanticAction::Focus => {
                node.attributes
                    .insert("data-oryn-focused".into(), String::new());
                ("focused element", true)
            }
            SemanticAction::Hover => ("dispatched pointer hover", false),
            SemanticAction::Submit if matches!(local_name, "form" | "button" | "input") => {
                ("dispatched form submission", false)
            }
            _ => {
                return Err(RuntimeError::UnsupportedAction {
                    action: action.action,
                });
            }
        };

        let revision_before = Revision(self.revision);
        self.revision += 1;
        self.next_action_id += 1;
        let upserted = if mutated {
            project_element(
                &document.dom,
                node_id,
                self.document_generation,
                node_id.slot + 1,
            )
            .into_iter()
            .collect()
        } else {
            Vec::new()
        };
        let effect = if mutated {
            Effect::DomMutation {
                summary: summary.into(),
            }
        } else {
            Effect::Event {
                event_type: match action.action {
                    SemanticAction::Hover => "mouseover",
                    SemanticAction::Submit => "submit",
                    _ => "click",
                }
                .into(),
                target: action.target,
            }
        };
        let result = ActionResult {
            contract_version: CONTRACT_VERSION,
            action_id: ActionId(self.next_action_id),
            revision_before,
            revision_after: Revision(self.revision),
            delta: Some(ObservationDelta {
                contract_version: CONTRACT_VERSION,
                from_revision: revision_before,
                to_revision: Revision(self.revision),
                upserted,
                removed: Vec::new(),
            }),
            effects: vec![effect],
            diagnostics: Vec::new(),
            execution_domain: ExecutionDomain::Native,
        };
        self.record_snapshot();
        let trace_kind = if mutated {
            TraceEventKind::Mutation {
                summary: summary.into(),
            }
        } else {
            TraceEventKind::EventDispatched {
                event_type: match action.action {
                    SemanticAction::Hover => "mouseover",
                    SemanticAction::Submit => "submit",
                    _ => "click",
                }
                .into(),
            }
        };
        self.trace
            .push(Revision(self.revision), Some(result.action_id), trace_kind);
        Ok(result)
    }

    #[cfg(feature = "v8-host")]
    fn execute_in_realm(&mut self, action: NativeAction) -> Result<ActionResult, RuntimeError> {
        let before = self.observation(ProjectionProfile::Compact);
        let node_id = crate::semantic::unpack_node_ref(action.target.node);
        let document = self
            .document
            .as_ref()
            .ok_or(RuntimeError::StaleSemanticRef)?;
        let node = document
            .dom
            .get(node_id)
            .map_err(|_| RuntimeError::StaleSemanticRef)?;
        if node.attributes.contains_key("disabled") {
            return Err(RuntimeError::Disabled);
        }
        let executable = self
            .executable
            .as_mut()
            .ok_or(RuntimeError::StaleSemanticRef)?;
        executable
            .apply_action(node_id, action.action, action.value.as_deref())
            .map_err(|error| RuntimeError::Script(error.to_string()))?;
        let console_entries = executable.take_console_entries().unwrap_or_default();
        let pending_navigation = executable.location().ok().filter(|location| {
            location != &self.url && Url::parse(location).ok() != Url::parse(&self.url).ok()
        });
        self.document = Some(executable.document.clone());
        let diagnostics = executable
            .diagnostics
            .iter()
            .filter_map(|diagnostic| diagnostic.detail.clone())
            .collect();

        let revision_before = Revision(self.revision);
        self.revision += 1;
        self.next_action_id += 1;
        let after = self.observation(ProjectionProfile::Compact);
        let before_by_ref = before
            .nodes
            .iter()
            .map(|node| (node.semantic_ref, node))
            .collect::<BTreeMap<_, _>>();
        let after_by_ref = after
            .nodes
            .iter()
            .map(|node| (node.semantic_ref, node))
            .collect::<BTreeMap<_, _>>();
        let delta = ObservationDelta {
            contract_version: CONTRACT_VERSION,
            from_revision: revision_before,
            to_revision: Revision(self.revision),
            upserted: after
                .nodes
                .iter()
                .filter(|node| before_by_ref.get(&node.semantic_ref).copied() != Some(*node))
                .cloned()
                .collect(),
            removed: before_by_ref
                .keys()
                .filter(|reference| !after_by_ref.contains_key(reference))
                .copied()
                .collect(),
        };
        let event_type = match action.action {
            SemanticAction::Hover => "mouseover",
            SemanticAction::Submit => "submit",
            SemanticAction::Focus => "focus",
            SemanticAction::Type => "input",
            SemanticAction::Clear => "input",
            SemanticAction::Check | SemanticAction::Uncheck | SemanticAction::Select => "change",
            _ => "click",
        };
        let mut effects = Vec::new();
        if !delta.upserted.is_empty() || !delta.removed.is_empty() {
            effects.push(Effect::DomMutation {
                summary: "script realm changed the native DOM".into(),
            });
        }
        effects.push(Effect::Event {
            event_type: event_type.into(),
            target: action.target,
        });
        let mut result = ActionResult {
            contract_version: CONTRACT_VERSION,
            action_id: ActionId(self.next_action_id),
            revision_before,
            revision_after: Revision(self.revision),
            delta: Some(delta),
            effects,
            diagnostics,
            execution_domain: ExecutionDomain::Native,
        };
        self.record_snapshot();
        self.trace.push(
            Revision(self.revision),
            Some(result.action_id),
            TraceEventKind::EventDispatched {
                event_type: event_type.into(),
            },
        );
        for entry in console_entries {
            self.trace.push(
                Revision(self.revision),
                Some(result.action_id),
                TraceEventKind::Console {
                    level: entry.level,
                    message: entry.message,
                },
            );
        }
        if result
            .delta
            .as_ref()
            .is_some_and(|delta| !delta.upserted.is_empty() || !delta.removed.is_empty())
        {
            self.trace.push(
                Revision(self.revision),
                Some(result.action_id),
                TraceEventKind::Mutation {
                    summary: "script realm changed the native DOM".into(),
                },
            );
        }
        if let Some(location) = pending_navigation {
            let navigation = self.navigate(location.clone(), HistoryUpdate::Push)?;
            result.revision_after = navigation.revision;
            result.delta = None;
            result.effects.push(Effect::Navigation { url: location });
        }
        Ok(result)
    }
}

fn visible_text_content(
    dom: &crate::dom::DomArena,
    id: crate::dom::NodeId,
) -> Result<String, crate::dom::DomError> {
    let node = dom.get(id)?;
    match &node.kind {
        crate::dom::NodeKind::Text { data } => Ok(data.clone()),
        crate::dom::NodeKind::Element { name }
            if matches!(
                name.local.as_ref(),
                "head" | "script" | "style" | "template" | "noscript"
            ) || is_hidden(dom, id) =>
        {
            Ok(String::new())
        }
        _ => {
            let mut result = String::new();
            for child in &node.children {
                result.push_str(&visible_text_content(dom, *child)?);
            }
            Ok(result)
        }
    }
}

fn selector_nodes<'a>(
    document: &'a ParsedDocument,
    selector: &'a str,
) -> impl Iterator<Item = crate::dom::NodeId> + 'a {
    document
        .dom
        .iter()
        .filter_map(move |(id, node)| matches_css_subject(node, selector).then_some(id))
}

fn matches_css_subject(node: &crate::dom::Node, selector: &str) -> bool {
    let crate::dom::NodeKind::Element { name } = &node.kind else {
        return false;
    };
    let subject = css_subject(selector);
    let (subject, excluded_class) = subject
        .split_once(":not(.")
        .map(|(base, excluded)| (base, Some(excluded.trim_end_matches(')'))))
        .unwrap_or((subject, None));
    if excluded_class.is_some_and(|excluded| has_class(node, excluded)) {
        return false;
    }
    let attribute_start = subject.find('[');
    let simple = &subject[..attribute_start.unwrap_or(subject.len())];
    let mut remainder = attribute_start.map(|index| &subject[index..]).unwrap_or("");
    while let Some(open) = remainder.find('[') {
        let after_open = &remainder[open + 1..];
        let Some(close) = after_open.find(']') else {
            return false;
        };
        let attribute = &after_open[..close];
        let (key, expected, contains) = if let Some((key, value)) = attribute.split_once("*=") {
            (key, Some(value.trim_matches(['\"', '\''])), true)
        } else if let Some((key, value)) = attribute.split_once('=') {
            (key, Some(value.trim_matches(['\"', '\''])), false)
        } else {
            (attribute, None, false)
        };
        let Some(actual) = node.attributes.get(key) else {
            return false;
        };
        if expected.is_some_and(|expected| {
            if contains {
                !actual.contains(expected)
            } else {
                actual != expected
            }
        }) {
            return false;
        }
        remainder = &after_open[close + 1..];
    }
    let mut tag_end = simple.len();
    for marker in ['#', '.'] {
        if let Some(index) = simple.find(marker) {
            tag_end = tag_end.min(index);
        }
    }
    let tag = &simple[..tag_end];
    if !tag.is_empty() && !name.local.as_ref().eq_ignore_ascii_case(tag) {
        return false;
    }
    if let Some(id_start) = simple.find('#') {
        let id = simple[id_start + 1..].split('.').next().unwrap_or_default();
        if node.attributes.get("id").is_none_or(|value| value != id) {
            return false;
        }
    }
    for class in simple.split('.').skip(1) {
        if !has_class(node, class) {
            return false;
        }
    }
    true
}

fn css_subject(selector: &str) -> &str {
    let mut bracket_depth = 0_u32;
    let mut quote = None;
    let mut subject_start = 0;
    for (index, character) in selector.char_indices() {
        if let Some(active_quote) = quote {
            if character == active_quote {
                quote = None;
            }
            continue;
        }
        match character {
            '"' | '\'' if bracket_depth > 0 => quote = Some(character),
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            character if character.is_whitespace() && bracket_depth == 0 => {
                subject_start = index + character.len_utf8();
            }
            _ => {}
        }
    }
    selector[subject_start..].trim()
}

fn has_class(node: &crate::dom::Node, class: &str) -> bool {
    node.attributes
        .get("class")
        .is_some_and(|classes| classes.split_whitespace().any(|item| item == class))
}

fn descendants_of(dom: &crate::dom::DomArena, root: crate::dom::NodeId) -> Vec<crate::dom::NodeId> {
    let mut result = Vec::new();
    let mut pending = dom
        .get(root)
        .map(|node| node.children.clone())
        .unwrap_or_default();
    while let Some(id) = pending.pop() {
        result.push(id);
        if let Ok(node) = dom.get(id) {
            pending.extend(node.children.iter().rev().copied());
        }
    }
    result
}

fn serialize_dom_node(dom: &crate::dom::DomArena, id: crate::dom::NodeId) -> String {
    let Ok(node) = dom.get(id) else {
        return String::new();
    };
    match &node.kind {
        crate::dom::NodeKind::Document | crate::dom::NodeKind::DocumentFragment => node
            .children
            .iter()
            .map(|child| serialize_dom_node(dom, *child))
            .collect(),
        crate::dom::NodeKind::DocumentType { name } => format!("<!DOCTYPE {name}>"),
        crate::dom::NodeKind::Text { data } => escape_html(data),
        crate::dom::NodeKind::Comment { data } => format!("<!--{data}-->"),
        crate::dom::NodeKind::Element { name } => {
            let attributes = node
                .attributes
                .iter()
                .map(|(name, value)| format!(" {name}=\"{}\"", escape_html(value)))
                .collect::<String>();
            let children = node
                .children
                .iter()
                .map(|child| serialize_dom_node(dom, *child))
                .collect::<String>();
            format!("<{0}{attributes}>{children}</{0}>", name.local)
        }
    }
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(feature = "v8-host")]
fn module_specifiers(source: &str) -> Vec<String> {
    source
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if !line.starts_with("import ") && !line.starts_with("export ") {
                return None;
            }
            let candidate = line
                .split_once(" from ")
                .map(|(_, value)| value)
                .or_else(|| line.strip_prefix("import "))?;
            let candidate = candidate.trim().trim_end_matches(';').trim();
            let quote = candidate.chars().next()?;
            matches!(quote, '\'' | '\"').then(|| {
                candidate[1..]
                    .split(quote)
                    .next()
                    .unwrap_or_default()
                    .to_string()
            })
        })
        .filter(|specifier| !specifier.is_empty())
        .collect()
}

#[cfg(feature = "v8-host")]
fn pinned_resource(url: &Url) -> Option<(&'static str, &'static str)> {
    match url.as_str() {
        "https://unpkg.com/react@18/umd/react.production.min.js" => Some((
            "https://unpkg.com/react@18.3.1/umd/react.production.min.js",
            "d949f1c3687aedadcedac85261865f29b17cd273997e7f6b2bfc53b2f9d4c4dd",
        )),
        "https://unpkg.com/react-dom@18/umd/react-dom.production.min.js" => Some((
            "https://unpkg.com/react-dom@18.3.1/umd/react-dom.production.min.js",
            "35f4f974f4b2bcd44da73963347f8952e341f83909e4498227d4e26b98f66f0d",
        )),
        "https://unpkg.com/@babel/standalone/babel.min.js" => Some((
            "https://unpkg.com/@babel/standalone@7.28.3/babel.min.js",
            "5c69309d4426c6d981f49291527eec59331a69c2c677eaf96fd10f81f015443f",
        )),
        _ => None,
    }
}

#[cfg(feature = "v8-host")]
fn origin_key(url: &Url) -> (String, Option<String>, Option<u16>) {
    (
        url.scheme().to_string(),
        url.host_str().map(str::to_ascii_lowercase),
        url.port_or_known_default(),
    )
}

fn document_title(document: &ParsedDocument) -> String {
    document
        .dom
        .iter()
        .find_map(|(id, node)| match &node.kind {
            crate::dom::NodeKind::Element { name } if name.local.as_ref() == "title" => {
                document.dom.text_content(id).ok()
            }
            _ => None,
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };

    use oryn_common::v2::{ExecutionDomain, ObservationSource};

    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn page_handle_queues_typed_operations() {
        let browser = Browser::new();
        let context = browser.new_context();
        let page = context.new_page();
        let observation = page.observe().await.expect("observe blank page");

        assert_eq!(observation.execution_domain, ExecutionDomain::Native);
        assert_eq!(observation.source, ObservationSource::NativeDom);
        page.close().await.expect("close page");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn loaded_html_produces_native_semantics() {
        let page = Browser::new().new_context().new_page();
        let revision = page
            .load_html(
                "https://example.test/form",
                "<title>Form</title><label>Email <input type=email required></label><button>Send</button>",
            )
            .await
            .expect("load HTML");
        let observation = page.observe().await.expect("observe document");

        assert_eq!(revision, Revision(1));
        assert_eq!(observation.page.title, "Form");
        assert_eq!(observation.page.document_generation, 1);
        assert!(observation.nodes.iter().any(|node| node.name == "Send"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn native_navigation_checks_redirects_and_supports_history() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
        let address = listener.local_addr().expect("fixture address");
        let server = std::thread::spawn(move || {
            for _ in 0..6 {
                let (mut stream, _) = listener.accept().expect("accept request");
                let mut request = [0_u8; 2048];
                let length = stream.read(&mut request).expect("read request");
                let request = String::from_utf8_lossy(&request[..length]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                let response = match path {
                    "/redirect" => concat!(
                        "HTTP/1.1 302 Found\r\n",
                        "Location: /one\r\n",
                        "Content-Length: 0\r\n",
                        "Connection: close\r\n\r\n"
                    )
                    .to_string(),
                    "/one" => fixture_response("<title>One</title><button>First</button>"),
                    "/two" => fixture_response("<title>Two</title><button>Second</button>"),
                    _ => fixture_response("<title>Unknown</title>"),
                };
                stream
                    .write_all(response.as_bytes())
                    .expect("write response");
            }
        });

        let page = Browser::new()
            .new_context_with_options(ContextOptions::loopback_test())
            .new_page();
        let first = page
            .goto(format!("http://{address}/redirect"))
            .await
            .expect("follow checked redirect");
        assert_eq!(first.status, 200);
        assert!(first.final_url.ends_with("/one"));
        assert_eq!(first.lifecycle.last(), Some(&LifecycleState::NetworkIdle));
        page.goto(format!("http://{address}/two"))
            .await
            .expect("second navigation");
        assert_eq!(page.back().await.expect("back").final_url, first.final_url);
        assert!(
            page.forward()
                .await
                .expect("forward")
                .final_url
                .ends_with("/two")
        );
        assert!(
            page.refresh()
                .await
                .expect("refresh")
                .final_url
                .ends_with("/two")
        );
        server.join().expect("fixture server");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn default_context_denies_loopback_navigation() {
        let page = Browser::new().new_context().new_page();
        let error = page
            .goto("http://127.0.0.1:9/")
            .await
            .expect_err("loopback must be denied before connecting");
        assert!(matches!(error, RuntimeError::Network(message) if message.contains("loopback")));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn ephemeral_context_shares_cookies_between_its_pages() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
        let address = listener.local_addr().expect("fixture address");
        let (cookie_sender, cookie_receiver) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            for index in 0..2 {
                let (mut stream, _) = listener.accept().expect("accept request");
                let mut request = [0_u8; 2048];
                let length = stream.read(&mut request).expect("read request");
                let request = String::from_utf8_lossy(&request[..length]);
                if index == 1 {
                    cookie_sender
                        .send(request.to_string())
                        .expect("capture cookie request");
                }
                let extra = if index == 0 {
                    "Set-Cookie: oryn_session=native; Path=/; SameSite=Lax\r\n"
                } else {
                    ""
                };
                let body = "<title>Cookie</title>";
                let response = format!(
                    "HTTP/1.1 200 OK\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write response");
            }
        });
        let context = Browser::new().new_context_with_options(ContextOptions::loopback_test());
        let first = context.new_page();
        first
            .goto(format!("http://{address}/set"))
            .await
            .expect("set cookie");
        first.close().await.expect("close first page");
        let second = context.new_page();
        second
            .goto(format!("http://{address}/check"))
            .await
            .expect("send cookie");
        let request = cookie_receiver.recv().expect("cookie request");
        assert!(
            request
                .to_ascii_lowercase()
                .contains("cookie: oryn_session=native")
        );
        server.join().expect("fixture server");
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn external_classic_scripts_execute_in_order_and_are_cached() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
        let address = listener.local_addr().expect("fixture address");
        let server = std::thread::spawn(move || {
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().expect("accept request");
                let mut request = [0_u8; 2048];
                let length = stream.read(&mut request).expect("read request");
                let request = String::from_utf8_lossy(&request[..length]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                let body = if path == "/app.js" {
                    "document.querySelector('#result').textContent='External loaded';"
                } else {
                    "<title>External</title><script src=/app.js></script><output id=result>Pending</output>"
                };
                stream
                    .write_all(fixture_response(body).as_bytes())
                    .expect("write response");
            }
        });
        let page = Browser::new()
            .new_context_with_options(ContextOptions::loopback_test())
            .new_page();
        page.goto(format!("http://{address}/"))
            .await
            .expect("navigate with external script");
        assert!(
            page.observe()
                .await
                .expect("observe")
                .nodes
                .iter()
                .any(|node| node.name == "External loaded")
        );
        page.refresh().await.expect("refresh using cached script");
        server.join().expect("fixture server");
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn external_modules_link_dependencies_and_evaluate() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
        let address = listener.local_addr().expect("fixture address");
        let server = std::thread::spawn(move || {
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().expect("accept request");
                let mut request = [0_u8; 2048];
                let length = stream.read(&mut request).expect("read request");
                let request = String::from_utf8_lossy(&request[..length]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                let body = match path {
                    "/entry.js" => {
                        "import { label } from './dep.js'; document.querySelector('#module-result').textContent=label;"
                    }
                    "/dep.js" => "export const label = 'Module loaded';",
                    _ => {
                        "<title>Module</title><script type=module src=/entry.js></script><output id=module-result>Pending</output>"
                    }
                };
                stream
                    .write_all(fixture_response(body).as_bytes())
                    .expect("write response");
            }
        });
        let page = Browser::new()
            .new_context_with_options(ContextOptions::loopback_test())
            .new_page();
        page.goto(format!("http://{address}/"))
            .await
            .expect("navigate with module graph");
        assert!(
            page.observe()
                .await
                .expect("observe")
                .nodes
                .iter()
                .any(|node| node.name == "Module loaded")
        );
        server.join().expect("fixture server");
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn promise_fetch_uses_the_policy_broker_and_updates_dom() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
        let address = listener.local_addr().expect("fixture address");
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().expect("accept request");
                let mut request = [0_u8; 2048];
                let length = stream.read(&mut request).expect("read request");
                let request = String::from_utf8_lossy(&request[..length]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                let body = if path == "/data" {
                    r#"{"label":"Fetched"}"#
                } else {
                    r#"<output id=result>Pending</output><script>fetch('/data').then(response=>response.json()).then(data=>document.querySelector('#result').textContent=data.label)</script>"#
                };
                stream
                    .write_all(fixture_response(body).as_bytes())
                    .expect("write response");
            }
        });
        let page = Browser::new()
            .new_context_with_options(ContextOptions::loopback_test())
            .new_page();
        page.goto(format!("http://{address}/"))
            .await
            .expect("navigate fetch fixture");
        assert!(
            page.observe()
                .await
                .expect("observe")
                .nodes
                .iter()
                .any(|node| node.name == "Fetched")
        );
        server.join().expect("fixture server");
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn same_origin_iframe_is_semantic_and_cross_origin_is_diagnostic() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
        let address = listener.local_addr().expect("fixture address");
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().expect("accept request");
                let mut request = [0_u8; 2048];
                let length = stream.read(&mut request).expect("read request");
                let request = String::from_utf8_lossy(&request[..length]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("/");
                let body = if path == "/frame" {
                    "<title>Child</title><button id=frame-action>Frame action</button>"
                } else {
                    "<iframe src=/frame></iframe><iframe src=https://cross-origin.invalid/frame></iframe>"
                };
                stream
                    .write_all(fixture_response(body).as_bytes())
                    .expect("write response");
            }
        });
        let page = Browser::new()
            .new_context_with_options(ContextOptions::loopback_test())
            .new_page();
        page.goto(format!("http://{address}/"))
            .await
            .expect("navigate frame fixture");
        let observation = page.observe().await.expect("observe frames");
        assert!(
            observation
                .nodes
                .iter()
                .any(|node| node.name == "Frame action")
        );
        assert!(observation.capabilities.iter().any(|capability| {
            capability.capability == "html.frame.cross_origin"
                && capability.support == SupportLevel::Unsupported
        }));
        server.join().expect("fixture server");
    }

    fn fixture_response(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    #[tokio::test(flavor = "current_thread")]
    async fn actions_are_generation_checked_and_report_effects() {
        let page = Browser::new().new_context().new_page();
        page.load_html(
            "https://example.test/form",
            "<label>Email<input aria-label=Email></label>",
        )
        .await
        .expect("load HTML");
        let observation = page.observe().await.expect("observe document");
        let input = observation
            .nodes
            .iter()
            .find(|node| node.actions.contains(&SemanticAction::Type))
            .expect("input semantic node");
        let result = page
            .execute(NativeAction {
                target: input.semantic_ref,
                action: SemanticAction::Type,
                value: Some("agent@example.test".into()),
            })
            .await
            .expect("type into input");

        assert_eq!(result.revision_before, Revision(1));
        assert_eq!(result.revision_after, Revision(2));
        assert!(matches!(result.effects[0], Effect::DomMutation { .. }));
        let delta = page
            .observe_delta(Revision(1))
            .await
            .expect("observe delta");
        assert_eq!(delta.to_revision, Revision(2));
        assert_eq!(delta.upserted.len(), 1);
        assert_eq!(
            delta.upserted[0].value.as_deref(),
            Some("agent@example.test")
        );
        let trace = page.trace().await.expect("read trace");
        assert!(matches!(
            trace.last().map(|event| &event.kind),
            Some(TraceEventKind::Mutation { .. })
        ));

        let error = page
            .execute(NativeAction {
                target: SemanticRef {
                    document_generation: 0,
                    node: input.semantic_ref.node,
                },
                action: SemanticAction::Clear,
                value: None,
            })
            .await
            .expect_err("old generation must fail");
        assert!(matches!(
            error,
            RuntimeError::WrongDocumentGeneration { .. }
        ));
    }

    #[cfg(feature = "v8-host")]
    #[tokio::test(flavor = "current_thread")]
    async fn typed_page_actions_dispatch_persistent_script_handlers() {
        let page = Browser::new().new_context().new_page();
        page.load_html(
            "https://example.test/svelte",
            include_str!("../../../test-harness/scenarios/spa/svelte-tasks.html"),
        )
        .await
        .expect("load executable fixture");
        let before = page.observe().await.expect("observe");
        let increment = before
            .nodes
            .iter()
            .find(|node| node.name == "Add item")
            .expect("increment button");
        page.execute(NativeAction {
            target: increment.semantic_ref,
            action: SemanticAction::Click,
            value: None,
        })
        .await
        .expect("click through page actor");
        let after = page.observe().await.expect("observe mutation");
        assert!(after.nodes.iter().any(|node| node.name == "Items: 1"));
    }
}
