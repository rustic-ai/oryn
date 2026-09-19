//! Native service-worker registration protocol. Creator identity is supplied by
//! the binding, never by a public JavaScript property or an author JSON bridge.
use crate::worker_wire::{AgentId, Creator, ScriptType, WorkerError};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RegistrationId(pub u64);
/// Logical version identity, independent of any process or context-local agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ServiceVersionId(pub [u8; 16]);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkerState {
    Installing,
    Installed,
    Activating,
    Activated,
    Redundant,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerSnapshot {
    pub version_id: ServiceVersionId,
    pub id: AgentId,
    pub script_url: String,
    pub state: WorkerState,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NavigationPreloadState {
    pub enabled: bool,
    pub header_value: String,
}
impl Default for NavigationPreloadState {
    fn default() -> Self {
        Self {
            enabled: false,
            header_value: "true".into(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NavigationPreloadOperation {
    Enable,
    Disable,
    SetHeaderValue(String),
    GetState,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrationSnapshot {
    pub id: RegistrationId,
    pub scope: String,
    pub update_via_cache: String,
    #[serde(default)]
    pub navigation_preload: NavigationPreloadState,
    pub installing: Option<WorkerSnapshot>,
    pub waiting: Option<WorkerSnapshot>,
    pub active: Option<WorkerSnapshot>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClientType {
    Window,
    DedicatedWorker,
    SharedWorker,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientSnapshot {
    pub id: String,
    pub url: String,
    pub client_type: ClientType,
    pub frame_type: String,
    pub focused: bool,
    pub visibility_state: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessageSource {
    Client {
        client: ClientSnapshot,
        origin: String,
    },
    ServiceWorker {
        worker: WorkerSnapshot,
        origin: String,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServiceRequest {
    NavigationPreload {
        creator: Creator,
        registration: RegistrationId,
        operation: NavigationPreloadOperation,
    },
    Cache {
        creator: Creator,
        operation: crate::cache_wire::CacheOperation,
    },
    PostMessage {
        creator: Creator,
        worker: ServiceVersionId,
        value: crate::message_wire::WireValue,
    },
    ClientPostMessage {
        client: String,
        value: crate::message_wire::WireValue,
    },
    ClientsGet {
        client: String,
    },
    ClientFocus {
        client: String,
    },
    ClientNavigate {
        client: String,
        url: String,
    },
    ClientsOpenWindow {
        url: String,
    },
    ClientsMatchAll {
        include_uncontrolled: bool,
        client_type: Option<ClientType>,
    },
    Register {
        creator: Creator,
        url: String,
        scope: Option<String>,
        script_type: ScriptType,
        update_via_cache: String,
    },
    Update {
        creator: Creator,
        registration: RegistrationId,
    },
    Unregister {
        creator: Creator,
        registration: RegistrationId,
    },
    GetRegistration {
        creator: Creator,
        url: String,
    },
    GetRegistrations {
        creator: Creator,
    },
    Ready {
        creator: Creator,
    },
    Poll {
        creator: Creator,
        operation: u64,
    },
    Acknowledge {
        creator: Creator,
        operation: u64,
    },
    Snapshot {
        creator: Creator,
        #[serde(default)]
        take_messages: bool,
    },
    Controller {
        creator: Creator,
    },
    WorkerRegistration,
    SkipWaiting,
    Claim,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServiceResult {
    NavigationPreload(NavigationPreloadState),
    WindowClient(Option<ClientSnapshot>),
    Client(Option<ClientSnapshot>),
    Clients(Vec<ClientSnapshot>),
    Registration(Option<RegistrationSnapshot>),
    Registrations(Vec<RegistrationSnapshot>),
    Bool(bool),
    Done,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientMessage {
    pub source: WorkerSnapshot,
    pub origin: String,
    pub value: crate::message_wire::WireValue,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerSnapshot {
    pub controller: Option<WorkerSnapshot>,
    pub registrations: Vec<RegistrationSnapshot>,
    pub messages: Vec<ClientMessage>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServiceResponse {
    Cache(crate::cache_wire::CacheResult),
    Pending {
        operation: u64,
    },
    Complete {
        operation: u64,
        result: Result<ServiceResult, WorkerError>,
    },
    Registration(RegistrationSnapshot),
    Controller(Option<WorkerSnapshot>),
    Snapshot(ContainerSnapshot),
    Done,
}
