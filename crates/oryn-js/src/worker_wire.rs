//! V8-free commands for parent-owned worker principals and isolated agent runtimes.
use crate::message_wire::{PortToken, WindowTarget, WireValue};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AgentId(pub u64);
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AgentClientId {
    pub id: u64,
    pub secret: [u8; 32],
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentKind {
    Dedicated,
    Shared,
    Service,
    AudioWorklet,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScriptType {
    Classic,
    Module,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImportedModuleType {
    JavaScript,
    Json,
    Text,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkerCredentials {
    Omit,
    SameOrigin,
    Include,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkerOptions {
    #[serde(default)]
    pub audio_sample_rate: Option<f64>,
    pub name: Vec<u16>,
    pub script_type: ScriptType,
    pub credentials: WorkerCredentials,
}
impl Default for WorkerOptions {
    fn default() -> Self {
        Self {
            audio_sample_rate: None,
            name: Vec::new(),
            script_type: ScriptType::Classic,
            credentials: WorkerCredentials::SameOrigin,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentOrigin {
    pub serialized: String,
    pub authority: String,
    pub opaque: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Creator {
    Window {
        target: WindowTarget,
        document_generation: u64,
        stream_epoch: u64,
    },
    Agent(AgentId),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCreation {
    pub kind: AgentKind,
    pub url: String,
    pub options: WorkerOptions,
    pub creator: Creator,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentLimits {
    pub heap_bytes: usize,
    pub queue_bytes: usize,
    pub queue_messages: usize,
    pub task_wall_ms: u64,
}
impl Default for AgentLimits {
    fn default() -> Self {
        Self {
            heap_bytes: 256 * 1024 * 1024,
            queue_bytes: 32 * 1024 * 1024,
            queue_messages: 1024,
            task_wall_ms: 30_000,
        }
    }
}
/// Native response policy snapshot. Blob/data workers inherit the creator's
/// policy self-origin and headers while retaining their actual document URL.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentPolicy {
    #[serde(default)]
    pub hide_nonces: bool,
    #[serde(default)]
    pub policy_sources: Vec<String>,
    pub url: String,
    #[serde(default)]
    pub policy_self_url: Option<String>,
    #[serde(default)]
    pub policy_self_urls: Vec<Option<String>>,
    /// Native origin used by newly parsed document meta policies. This is
    /// distinct from each inherited policy's original self origin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_self_url: Option<String>,
    pub headers: Vec<(String, String)>,
}
impl AgentPolicy {
    pub fn with_document_url(mut self, url: String) -> Self {
        if self.url != url && self.policy_self_url.is_none() {
            self.policy_self_url = Some(self.url.clone());
        }
        if self.url != url && !matches!(url.split('#').next(), Some("about:blank" | "about:srcdoc"))
        {
            self.document_self_url = None;
        }
        self.url = url;
        self
    }
}
/// Durable provenance of a response admitted by the native script loader.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentImportKind {
    Classic,
    JavaScript,
    Json,
    Text,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentImportMetadata {
    pub kind: AgentImportKind,
    pub response_url: String,
    pub content_type: Option<String>,
    pub referrer_policy: String,
    pub request_referrer_source: Option<String>,
    pub request_referrer_policy: String,
}
/// Produced by the parent's authorized script loader, never by author JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentBootstrap {
    #[serde(default)]
    pub import_metadata: BTreeMap<String, AgentImportMetadata>,
    #[serde(default)]
    pub service_skip_waiting: bool,
    /// Parent-computed raw representation hashes for service update comparisons.
    #[serde(default)]
    pub resource_digests: BTreeMap<String, [u8; 32]>,
    #[serde(default)]
    pub secure_context: bool,
    #[serde(default)]
    pub service_registration: Option<crate::service_worker_wire::RegistrationId>,
    #[serde(default)]
    pub service_controller: Option<crate::service_worker_wire::RegistrationId>,
    pub id: AgentId,
    pub kind: AgentKind,
    pub partition: String,
    pub origin: AgentOrigin,
    pub script_url: String,
    pub source: Vec<u16>,
    pub modules: BTreeMap<String, Vec<u16>>,
    pub options: WorkerOptions,
    pub referrer_policy: String,
    pub limits: AgentLimits,
    pub policy: AgentPolicy,
    #[serde(default)]
    pub top_level_url: Option<String>,
    #[serde(default)]
    pub cookie_ancestor_urls: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConnection {
    pub agent: AgentId,
    pub client: AgentClientId,
    pub port: Option<PortToken>,
}
/// Native loader provenance; author-visible exception names are not authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScriptFailureKind {
    Fetch,
    Infrastructure,
    Cancelled,
    Capacity,
    Integrity,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerError {
    pub name: String,
    pub message: String,
    #[serde(default)]
    pub script_failure: Option<ScriptFailureKind>,
}
impl WorkerError {
    pub fn new(name: &str, message: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            message: message.into(),
            script_failure: None,
        }
    }
    pub fn with_script_failure(mut self, kind: ScriptFailureKind) -> Self {
        self.script_failure = Some(kind);
        self
    }
}
impl std::fmt::Display for WorkerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.name, self.message)
    }
}
impl std::error::Error for WorkerError {}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentEvent {
    Ready,
    Message {
        client: Option<AgentClientId>,
        value: WireValue,
    },
    Error {
        message: Vec<u16>,
        filename: String,
        line: u32,
        column: u32,
    },
    LifecycleFinished {
        id: u64,
        result: Result<(), WorkerError>,
    },
    FetchResponse {
        id: u64,
        response: Result<AgentFetchResponse, WorkerError>,
    },
    FetchFallback {
        id: u64,
    },
    FetchResponseStart {
        id: u64,
        response: AgentFetchResponse,
    },
    FetchResponseChunk {
        id: u64,
        bytes: Vec<u8>,
    },
    FetchResponseEnd {
        id: u64,
        result: Result<(), WorkerError>,
    },
    Closed,
    WorkletFinished {
        id: u64,
        result: Result<(), WorkerError>,
    },
    WorkletDefinitions(Vec<String>),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentFetchRequest {
    #[serde(default)]
    pub keepalive: bool,
    #[serde(default)]
    pub privileged_range: bool,
    #[serde(default)]
    pub cache: crate::platform::FetchCacheMode,
    #[serde(default)]
    pub navigation: crate::platform::NavigationRequestFlags,
    #[serde(default)]
    pub integrity: String,
    #[serde(default)]
    pub navigation_preload: bool,
    #[serde(default)]
    pub streaming: bool,
    pub id: u64,
    pub client_id: String,
    pub url: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub destination: String,
    pub mode: String,
    pub credentials: String,
    #[serde(default = "default_redirect")]
    pub redirect: String,
}
fn default_redirect() -> String {
    "follow".into()
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentFetchResponse {
    #[serde(default)]
    pub timing: Option<crate::platform::ResourceTimingMetadata>,
    #[serde(default)]
    pub redirected: bool,
    #[serde(default)]
    pub range_requested: bool,
    pub status: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub response_url: String,
    pub response_type: String,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum LifecycleEvent {
    Install,
    Activate,
    Message,
    Push,
    Sync,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkletCommand {
    LoadModule {
        id: u64,
        url: String,
        credentials: WorkerCredentials,
    },
    Construct {
        id: u64,
        name: String,
        value: Box<WireValue>,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentCommand {
    ServiceMessage {
        id: u64,
        source: crate::service_worker_wire::MessageSource,
        value: WireValue,
    },
    Message {
        client: Option<AgentClientId>,
        value: WireValue,
    },
    Connect {
        client: AgentClientId,
        port: PortToken,
    },
    Lifecycle {
        id: u64,
        event: LifecycleEvent,
        payload: Option<WireValue>,
    },
    Fetch(AgentFetchRequest),
    CancelTask {
        id: u64,
    },
    FetchPull {
        id: u64,
    },
    Stop,
    Worklet(WorkletCommand),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentServiceRequest {
    TrustedTypes(crate::trusted_types::PolicyCheck),
    CspEvents {
        after: u64,
    },
    SharedMemory {
        creator: Creator,
        request: crate::shared_memory::SharedMemoryRequest,
    },
    PublishSecureContext {
        creator: Creator,
        secure: bool,
    },
    Service(crate::service_worker_wire::ServiceRequest),
    BlobRegister {
        creator: Creator,
        bytes: Vec<u8>,
        media_type: String,
    },
    BlobResolve {
        creator: Creator,
        url: String,
    },
    BlobRevoke {
        creator: Creator,
        url: String,
    },
    Create(AgentCreation),
    Post {
        connection: AgentConnection,
        value: WireValue,
    },
    Poll {
        connection: AgentConnection,
        max: usize,
    },
    Terminate {
        connection: AgentConnection,
    },
    Release {
        connection: AgentConnection,
    },
    ImportScript {
        url: String,
        module: bool,
    },
    ImportModule {
        url: String,
        kind: ImportedModuleType,
        referrer_source: String,
        referrer_policy: String,
    },
    Message(crate::message_wire::MessageRequest),
    PlatformSubmit {
        operation_id: crate::platform::HostOperationId,
        request: crate::platform::PlatformHostRequest,
    },
    PlatformPoll {
        max: usize,
    },
    PlatformCancel {
        operation_id: crate::platform::HostOperationId,
    },
    PlatformRetire,
    PlatformReleaseResult {
        stream_id: Option<u64>,
        socket_id: Option<u64>,
    },
    Worklet {
        connection: AgentConnection,
        command: WorkletCommand,
    },
    ImportWorkletModule {
        url: String,
        credentials: WorkerCredentials,
        referrer_source: String,
        referrer_policy: String,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentServiceResponse {
    TrustedTypes {
        allowed: bool,
    },
    CspEvents(Vec<crate::platform_control::CspEvent>),
    SharedMemory(crate::shared_memory::SharedMemoryResponse),
    Service(crate::service_worker_wire::ServiceResponse),
    BlobUrl(String),
    Blob {
        bytes: Vec<u8>,
        media_type: String,
    },
    Connected(AgentConnection),
    Events(Vec<AgentEvent>),
    Script {
        url: String,
        source: Vec<u16>,
        referrer_policy: String,
    },
    Message(crate::message_wire::MessageResponse),
    PlatformCompletions(Vec<crate::platform::PlatformHostCompletion>),
    Done,
}
/// Native-only response rollback metadata; never supplied through author JSON.
#[derive(Clone, Default)]
pub struct ServiceResponseScope {
    pub memory_creator: Option<Creator>,
    pub wait_admission: Option<crate::shared_memory::WaitAdmissionId>,
}
impl ServiceResponseScope {
    pub fn capture(request: &AgentServiceRequest) -> Self {
        match request {
            AgentServiceRequest::SharedMemory { creator, request } => Self {
                memory_creator: Some(creator.clone()),
                wait_admission: match request {
                    crate::shared_memory::SharedMemoryRequest::WaitAdmission { id, .. } => {
                        Some(*id)
                    }
                    _ => None,
                },
            },
            _ => Self::default(),
        }
    }
}
/// A route-bound principal. Implementations supply real context/agent identity;
/// methods do not accept arbitrary page IDs or partition strings from scripts.
pub trait WorkerTransport: Send + Sync {
    fn request(&self, request: AgentServiceRequest) -> Result<AgentServiceResponse, WorkerError>;
    /// Parent-side IPC calls this only for an authentic response that did not
    /// publish. Implementations retain failed cleanup instead of dropping it.
    fn reclaim_response(
        &self,
        _scope: ServiceResponseScope,
        _response: &AgentServiceResponse,
    ) -> Result<(), WorkerError> {
        Err(WorkerError::new(
            "InvalidStateError",
            "response rollback unavailable",
        ))
    }
    /// Revoke and wake in-flight broker waits without entering V8 or waiting
    /// for a request mutex. Called before an owned agent process is joined.
    fn retire(&self) {}
    /// Wait for native parent operations after revocation; retain ownership on
    /// timeout so the process registry can retry teardown.
    fn drain(&self, _timeout: std::time::Duration) -> Result<(), WorkerError> {
        Ok(())
    }
}

#[cfg(test)]
mod authority_schema_regressions {
    use super::*;
    #[test]
    fn window_creator_never_defaults_native_lifetime_fields() {
        for value in [
            serde_json::json!({"Window":{"target":"Root","document_generation":1}}),
            serde_json::json!({"Window":{"target":"Root","stream_epoch":0}}),
            serde_json::json!({"Window":{"document_generation":1,"stream_epoch":0}}),
        ] {
            assert!(serde_json::from_value::<Creator>(value).is_err());
        }
        let creator = Creator::Window {
            target: WindowTarget::Root,
            document_generation: 9,
            stream_epoch: 7,
        };
        let copy: Creator =
            serde_json::from_value(serde_json::to_value(&creator).unwrap()).unwrap();
        assert_eq!(copy, creator);
    }
    #[test]
    fn bootstrap_requires_explicit_native_policy_not_an_allow_all_default() {
        let mut value = serde_json::json!({"id":1,"kind":"Dedicated","partition":"native-partition","origin":{"serialized":"https://worker.test","authority":"native-origin","opaque":false},"script_url":"https://worker.test/worker.js","source":[],"modules":{},"options":WorkerOptions::default(),"referrer_policy":"no-referrer","limits":AgentLimits::default(),"policy":AgentPolicy::default()});
        assert!(serde_json::from_value::<AgentBootstrap>(value.clone()).is_ok());
        value.as_object_mut().unwrap().remove("policy");
        assert!(serde_json::from_value::<AgentBootstrap>(value).is_err());
        assert!(
            serde_json::from_value::<AgentOrigin>(
                serde_json::json!({"serialized":"https://worker.test","opaque":false})
            )
            .is_err()
        );
    }
    #[test]
    fn fractional_audio_sample_rate_remains_a_partial_equality_wire_value() {
        let options = WorkerOptions {
            audio_sample_rate: Some(44100.5),
            ..Default::default()
        };
        let copy: WorkerOptions =
            serde_json::from_value(serde_json::to_value(&options).unwrap()).unwrap();
        assert_eq!(options, copy);
    }
}

#[cfg(test)]
mod policy_document_origin_tests {
    use super::AgentPolicy;

    #[test]
    fn legacy_policy_wire_defaults_native_document_origin_without_changing_bytes() {
        let legacy = serde_json::json!({"url":"https://a.test/path","headers":[]});
        let policy: AgentPolicy = serde_json::from_value(legacy).unwrap();
        assert!(policy.document_self_url.is_none());
        assert!(
            serde_json::to_value(&policy)
                .unwrap()
                .get("document_self_url")
                .is_none()
        );
    }

    #[test]
    fn rebasing_retains_blank_origin_but_does_not_grant_it_to_data_or_http() {
        let policy = AgentPolicy {
            url: "https://a.test/path".into(),
            document_self_url: Some("https://a.test".into()),
            ..Default::default()
        };
        for target in ["about:blank", "about:blank#fragment", "about:srcdoc"] {
            let inherited = policy.clone().with_document_url(target.into());
            assert_eq!(
                inherited.document_self_url.as_deref(),
                Some("https://a.test")
            );
            assert_eq!(
                inherited.policy_self_url.as_deref(),
                Some("https://a.test/path")
            );
        }
        for target in ["data:text/html,new", "https://b.test/next"] {
            assert!(
                policy
                    .clone()
                    .with_document_url(target.into())
                    .document_self_url
                    .is_none()
            );
        }
    }
}
