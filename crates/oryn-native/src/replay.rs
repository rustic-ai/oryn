//! Policy-aware network record/replay and versioned context checkpoints.

use std::sync::{Arc, Mutex};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::network::{
    NetworkBroker, NetworkError, NetworkEvent, NetworkOperation, NetworkRequest, NetworkResponse,
};
use crate::{
    execution::{FormValueState, OriginStorageState},
    trace::TraceEvent,
};
use oryn_common::v3::{PageId, PageLifecycleEvent, Revision};
use std::collections::BTreeMap;

pub const TRACE_ARCHIVE_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body_sha256: String,
    pub body_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body_sha256: String,
    pub body_bytes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_base64: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedExchange {
    pub sequence: u64,
    pub request: RecordedRequest,
    pub response: RecordedResponse,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkArchive {
    pub schema_version: u16,
    pub capture_response_bodies: bool,
    pub exchanges: Vec<RecordedExchange>,
}

impl Default for NetworkArchive {
    fn default() -> Self {
        Self {
            schema_version: TRACE_ARCHIVE_VERSION,
            capture_response_bodies: true,
            exchanges: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReplayMode {
    Live,
    Record,
    Replay,
}

#[derive(Debug)]
struct ReplayState {
    mode: ReplayMode,
    archive: NetworkArchive,
    cursor: usize,
}

#[derive(Debug, Clone)]
pub struct NetworkReplayController(Arc<Mutex<ReplayState>>);

impl Default for NetworkReplayController {
    fn default() -> Self {
        Self::live()
    }
}

impl NetworkReplayController {
    pub fn live() -> Self {
        Self(Arc::new(Mutex::new(ReplayState {
            mode: ReplayMode::Live,
            archive: NetworkArchive::default(),
            cursor: 0,
        })))
    }

    pub fn recording(capture_response_bodies: bool) -> Self {
        Self(Arc::new(Mutex::new(ReplayState {
            mode: ReplayMode::Record,
            archive: NetworkArchive {
                capture_response_bodies,
                ..NetworkArchive::default()
            },
            cursor: 0,
        })))
    }

    pub fn replay(archive: NetworkArchive) -> Result<Self, NetworkError> {
        if archive.schema_version != TRACE_ARCHIVE_VERSION {
            return Err(NetworkError::Replay(format!(
                "unsupported network archive version {}",
                archive.schema_version
            )));
        }
        Ok(Self(Arc::new(Mutex::new(ReplayState {
            mode: ReplayMode::Replay,
            archive,
            cursor: 0,
        }))))
    }

    pub fn archive(&self) -> NetworkArchive {
        self.0.lock().expect("network replay lock").archive.clone()
    }

    pub fn consumed(&self) -> usize {
        self.0.lock().expect("network replay lock").cursor
    }

    pub fn is_replay(&self) -> bool {
        self.0.lock().expect("network replay lock").mode == ReplayMode::Replay
    }

    pub fn wrap<B: NetworkBroker + 'static>(&self, inner: B) -> ControlledNetworkBroker {
        ControlledNetworkBroker {
            controller: self.clone(),
            inner: Box::new(inner),
        }
    }
}

pub struct ControlledNetworkBroker {
    controller: NetworkReplayController,
    inner: Box<dyn NetworkBroker>,
}

impl NetworkBroker for ControlledNetworkBroker {
    fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        let mode = self
            .controller
            .0
            .lock()
            .map_err(|_| NetworkError::Replay("network replay lock poisoned".into()))?
            .mode;
        match mode {
            ReplayMode::Live => self.inner.send(request),
            ReplayMode::Record => {
                let response = self.inner.send(request.clone())?;
                let mut state = self
                    .controller
                    .0
                    .lock()
                    .map_err(|_| NetworkError::Replay("network replay lock poisoned".into()))?;
                let sequence = state.archive.exchanges.len() as u64;
                let capture = state.archive.capture_response_bodies;
                state.archive.exchanges.push(RecordedExchange {
                    sequence,
                    request: recorded_request(&request),
                    response: recorded_response(&response, capture),
                });
                Ok(response)
            }
            ReplayMode::Replay => {
                let mut state = self
                    .controller
                    .0
                    .lock()
                    .map_err(|_| NetworkError::Replay("network replay lock poisoned".into()))?;
                let Some(exchange) = state.archive.exchanges.get(state.cursor).cloned() else {
                    return Err(NetworkError::Replay(format!(
                        "network archive exhausted at request {} {}",
                        request.method, request.url
                    )));
                };
                let actual = recorded_request(&request);
                if exchange.request != actual {
                    return Err(NetworkError::Replay(format!(
                        "network replay mismatch at exchange {}: expected {} {}, got {} {}",
                        exchange.sequence,
                        exchange.request.method,
                        exchange.request.url,
                        actual.method,
                        actual.url
                    )));
                }
                let body = exchange
                    .response
                    .body_base64
                    .as_deref()
                    .ok_or_else(|| {
                        NetworkError::Replay(format!(
                            "exchange {} omitted its response body and cannot be replayed",
                            exchange.sequence
                        ))
                    })
                    .and_then(|body| {
                        BASE64
                            .decode(body)
                            .map_err(|error| NetworkError::Replay(error.to_string()))
                    })?;
                if sha256(&body) != exchange.response.body_sha256 {
                    return Err(NetworkError::Replay(format!(
                        "exchange {} response body hash mismatch",
                        exchange.sequence
                    )));
                }
                state.cursor += 1;
                Ok(NetworkResponse {
                    response_policy: None,
                    timing: None,
                    request_id: request.request_id,
                    status: exchange.response.status,
                    headers: exchange.response.headers,
                    body,
                })
            }
        }
    }

    fn start_operation(&self, request: NetworkRequest) -> Result<NetworkOperation, NetworkError> {
        let mode = self
            .controller
            .0
            .lock()
            .map_err(|_| NetworkError::Replay("network replay lock poisoned".into()))?
            .mode;
        match mode {
            ReplayMode::Live => self.inner.start_operation(request),
            ReplayMode::Replay => {
                let response = self.send(request)?;
                let request_id = response.request_id;
                let (sender, events) = tokio::sync::mpsc::channel(3);
                let (demand, mut requested) = tokio::sync::mpsc::channel(1);
                std::thread::spawn(move || {
                    let length = response.body.len();
                    if sender
                        .blocking_send(NetworkEvent::Headers {
                            response_policy: None,
                            timing: None,
                            request_id,
                            status: response.status,
                            headers: response.headers,
                            content_length: Some(length),
                            upload: None,
                        })
                        .is_err()
                    {
                        return;
                    }
                    if !response.body.is_empty() && requested.blocking_recv().is_some() {
                        let _ = sender.blocking_send(NetworkEvent::Chunk {
                            request_id,
                            bytes: bytes::Bytes::from(response.body),
                        });
                    }
                    let _ = sender.blocking_send(NetworkEvent::Complete {
                        encoded_body_size: None,
                        request_id,
                    });
                });
                Ok(NetworkOperation {
                    body_demand_pending: Default::default(),
                    prefetched: Default::default(),
                    request_id,
                    events,
                    demand: Some(demand),
                })
            }
            ReplayMode::Record => {
                let request_id = request.request_id;
                let recorded = request.clone();
                let controller = self.controller.clone();
                let mut operation = self.inner.start_operation(request)?;
                let demand = operation.demand.take();
                let (sender, events) = tokio::sync::mpsc::channel(8);
                std::thread::spawn(move || {
                    let mut status = None;
                    let mut headers = Vec::new();
                    let mut body = Vec::new();
                    while let Some(event) = operation.blocking_event() {
                        match &event {
                            NetworkEvent::Headers {
                                status: value,
                                headers: values,
                                ..
                            } => {
                                status = Some(*value);
                                headers = values.clone();
                            }
                            NetworkEvent::Chunk { bytes, .. } => body.extend_from_slice(bytes),
                            NetworkEvent::Complete { .. } => {
                                if let Some(status) = status
                                    && let Ok(mut state) = controller.0.lock()
                                {
                                    let response = NetworkResponse {
                                        response_policy: None,
                                        timing: None,
                                        request_id,
                                        status,
                                        headers: headers.clone(),
                                        body: body.clone(),
                                    };
                                    let sequence = state.archive.exchanges.len() as u64;
                                    let capture = state.archive.capture_response_bodies;
                                    state.archive.exchanges.push(RecordedExchange {
                                        sequence,
                                        request: recorded_request(&recorded),
                                        response: recorded_response(&response, capture),
                                    });
                                }
                            }
                            NetworkEvent::Failed { .. } => {}
                        }
                        if sender.blocking_send(event).is_err() {
                            break;
                        }
                    }
                });
                Ok(NetworkOperation {
                    body_demand_pending: Default::default(),
                    prefetched: Default::default(),
                    request_id,
                    events,
                    demand,
                })
            }
        }
    }

    fn cancel_operation(&self, request_id: u64) -> Result<(), NetworkError> {
        self.inner.cancel_operation(request_id)
    }
}

fn recorded_request(request: &NetworkRequest) -> RecordedRequest {
    RecordedRequest {
        method: request.method.to_ascii_uppercase(),
        url: request.url.clone(),
        headers: scrub_request_headers(&request.headers),
        body_sha256: sha256(&request.body),
        body_bytes: request.body.len(),
    }
}

fn recorded_response(response: &NetworkResponse, capture_body: bool) -> RecordedResponse {
    RecordedResponse {
        status: response.status,
        headers: scrub_response_headers(&response.headers),
        body_sha256: sha256(&response.body),
        body_bytes: response.body.len(),
        body_base64: capture_body.then(|| BASE64.encode(&response.body)),
    }
}

fn scrub_request_headers(headers: &[(String, String)]) -> Vec<(String, String)> {
    scrub_headers(
        headers,
        &[
            "authorization",
            "cookie",
            "proxy-authorization",
            "x-api-key",
        ],
    )
}

fn scrub_response_headers(headers: &[(String, String)]) -> Vec<(String, String)> {
    scrub_headers(headers, &["set-cookie", "set-cookie2", "www-authenticate"])
}

fn scrub_headers(headers: &[(String, String)], denied: &[&str]) -> Vec<(String, String)> {
    let mut result = headers
        .iter()
        .filter(|(name, _)| !denied.contains(&name.to_ascii_lowercase().as_str()))
        .cloned()
        .collect::<Vec<_>>();
    result.sort_by(|left, right| {
        left.0
            .to_ascii_lowercase()
            .cmp(&right.0.to_ascii_lowercase())
            .then_with(|| left.1.cmp(&right.1))
    });
    result
}

fn sha256(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeterminismConfig {
    pub unix_time_ms: i64,
    pub random_seed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NondeterministicRegion {
    WebSocket,
    LiveExternalProcess,
    UserBrowser,
    HardwareCredential,
    Media,
    UncapturedResponseBody,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckpointPage {
    pub source_page_id: PageId,
    pub opener_page_id: Option<PageId>,
    pub url: String,
    pub active: bool,
    pub storage: Vec<OriginStorageState>,
    pub form_values: Vec<FormValueState>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckpointPayload {
    pub schema_version: u16,
    pub name: String,
    pub context_id: u64,
    pub pages: Vec<CheckpointPage>,
    pub task_metadata: BTreeMap<String, String>,
    pub determinism: Option<DeterminismConfig>,
    pub network: NetworkArchive,
    /// First intent sequence that must be replayed after this checkpoint.
    pub intent_cursor: u64,
    pub nondeterministic_regions: Vec<NondeterministicRegion>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextCheckpoint {
    pub payload: CheckpointPayload,
    pub sha256: String,
}

impl ContextCheckpoint {
    pub fn seal(payload: CheckpointPayload) -> Result<Self, serde_json::Error> {
        let sha256 = sha256(&serde_json::to_vec(&payload)?);
        Ok(Self { payload, sha256 })
    }

    pub fn verify(&self) -> Result<(), NetworkError> {
        if self.payload.schema_version != TRACE_ARCHIVE_VERSION {
            return Err(NetworkError::Replay(format!(
                "unsupported checkpoint version {}",
                self.payload.schema_version
            )));
        }
        let actual = serde_json::to_vec(&self.payload)
            .map(|bytes| sha256(&bytes))
            .map_err(|error| NetworkError::Replay(error.to_string()))?;
        if actual != self.sha256 {
            return Err(NetworkError::Replay(
                "checkpoint content hash does not match its payload".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoredPage {
    pub source_page_id: PageId,
    pub destination_page_id: PageId,
    pub destination_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointRestore {
    pub checkpoint_sha256: String,
    pub pages: Vec<RestoredPage>,
    pub active_page_id: Option<PageId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayBranch {
    pub branch_id: String,
    pub parent_checkpoint_sha256: String,
    pub strategy: String,
    pub restored_pages: Vec<RestoredPage>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskTraceArchive {
    pub schema_version: u16,
    pub context_id: u64,
    pub determinism: Option<DeterminismConfig>,
    pub page_events: Vec<PageLifecycleEvent>,
    pub page_traces: BTreeMap<String, Vec<TraceEvent>>,
    pub intents: Vec<IntentTraceRecord>,
    #[serde(default)]
    pub domain_decisions: Vec<crate::execution::DomainDecision>,
    pub network: NetworkArchive,
    pub nondeterministic_regions: Vec<NondeterministicRegion>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntentTraceRecord {
    pub sequence: u64,
    pub page_id: Option<PageId>,
    pub input: String,
    pub revision_before: Option<Revision>,
    pub revision_after: Option<Revision>,
    pub outcome: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayDivergenceKind {
    MissingPageMapping,
    RedactedInput,
    ExecutionFailure,
    OutcomeMismatch,
    UnconsumedNetwork,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayDivergence {
    pub sequence: u64,
    pub kind: ReplayDivergenceKind,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayRun {
    pub checkpoint_sha256: String,
    pub trace_schema_version: u16,
    pub attempted_intents: u64,
    pub replayed_intents: u64,
    pub consumed_network_exchanges: usize,
    pub expected_network_exchanges: usize,
    pub deterministic: bool,
    pub divergences: Vec<ReplayDivergence>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FailureArtifactPayload {
    pub schema_version: u16,
    pub failure_id: String,
    pub checkpoint: ContextCheckpoint,
    pub trace: TaskTraceArchive,
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FailureArtifact {
    pub payload: FailureArtifactPayload,
    pub sha256: String,
}

impl FailureArtifact {
    pub fn seal(payload: FailureArtifactPayload) -> Result<Self, serde_json::Error> {
        let sha256 = sha256(&serde_json::to_vec(&payload)?);
        Ok(Self { payload, sha256 })
    }

    pub fn verify(&self) -> Result<(), NetworkError> {
        if self.payload.schema_version != TRACE_ARCHIVE_VERSION
            || self.payload.trace.schema_version != TRACE_ARCHIVE_VERSION
        {
            return Err(NetworkError::Replay(
                "unsupported failed-task artifact version".into(),
            ));
        }
        self.payload.checkpoint.verify()?;
        let actual = serde_json::to_vec(&self.payload)
            .map(|bytes| sha256(&bytes))
            .map_err(|error| NetworkError::Replay(error.to_string()))?;
        if actual != self.sha256 {
            return Err(NetworkError::Replay(
                "failure artifact content hash does not match its payload".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{network::PolicyBroker, security::NetworkPolicy};

    struct FixtureBroker;

    impl NetworkBroker for FixtureBroker {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            Ok(NetworkResponse {
                response_policy: None,
                timing: None,
                request_id: request.request_id,
                status: 200,
                headers: vec![
                    ("content-type".into(), "text/plain".into()),
                    ("set-cookie".into(), "secret=yes".into()),
                ],
                body: b"captured".to_vec(),
            })
        }
    }

    fn request(id: u64) -> NetworkRequest {
        NetworkRequest {
            request_id: id,
            method: "GET".into(),
            url: "https://example.test/data".into(),
            headers: vec![
                ("accept".into(), "text/plain".into()),
                ("authorization".into(), "Bearer secret".into()),
            ],
            body: Vec::new(),
            credentials: true,
            security: crate::cookies::CookieRequestContext::default(),
            resolved_addrs: Vec::new(),
        }
    }

    #[test]
    fn recorded_network_is_scrubbed_and_replays_exactly() {
        let recorder = NetworkReplayController::recording(true);
        let broker = recorder.wrap(FixtureBroker);
        let response = broker.send(request(1)).expect("record request");
        assert_eq!(response.body, b"captured");
        let archive = recorder.archive();
        assert_eq!(archive.exchanges.len(), 1);
        assert!(
            archive.exchanges[0]
                .request
                .headers
                .iter()
                .all(|(name, _)| name != "authorization")
        );
        assert!(
            archive.exchanges[0]
                .response
                .headers
                .iter()
                .all(|(name, _)| name != "set-cookie")
        );

        let replay = NetworkReplayController::replay(archive).expect("valid archive");
        let broker = replay.wrap(FixtureBroker);
        let response = broker.send(request(99)).expect("replayed request");
        assert_eq!(response.request_id, 99);
        assert_eq!(response.body, b"captured");
        assert_eq!(replay.consumed(), 1);
    }

    #[test]
    fn replay_rejects_divergent_requests() {
        let recorder = NetworkReplayController::recording(true);
        let broker = recorder.wrap(FixtureBroker);
        broker.send(request(1)).expect("record request");
        let replay = NetworkReplayController::replay(recorder.archive()).expect("valid archive");
        let broker = replay.wrap(FixtureBroker);
        let mut divergent = request(2);
        divergent.url = "https://example.test/other".into();
        assert!(matches!(
            broker.send(divergent),
            Err(NetworkError::Replay(_))
        ));
        assert_eq!(replay.consumed(), 0);
    }

    #[test]
    fn current_network_policy_is_enforced_before_captured_replay() {
        let recorder = NetworkReplayController::recording(true);
        let recording = recorder.wrap(FixtureBroker);
        let mut loopback = request(1);
        loopback.url = "http://127.0.0.1/private".into();
        recording.send(loopback.clone()).expect("record fixture");
        let replay = NetworkReplayController::replay(recorder.archive()).expect("archive");
        let broker = PolicyBroker::new(NetworkPolicy::default(), replay.wrap(FixtureBroker));
        assert!(matches!(
            broker.send(loopback),
            Err(NetworkError::Policy(_))
        ));
        assert_eq!(replay.consumed(), 0);
    }

    #[test]
    fn replay_rejects_tampered_response_body() {
        let recorder = NetworkReplayController::recording(true);
        recorder
            .wrap(FixtureBroker)
            .send(request(1))
            .expect("record fixture");
        let mut archive = recorder.archive();
        archive.exchanges[0].response.body_base64 = Some(BASE64.encode(b"tampered"));
        let replay = NetworkReplayController::replay(archive).expect("archive");
        assert!(matches!(
            replay.wrap(FixtureBroker).send(request(2)),
            Err(NetworkError::Replay(_))
        ));
    }

    #[test]
    fn sealed_checkpoint_rejects_payload_tampering() {
        let mut checkpoint = ContextCheckpoint::seal(CheckpointPayload {
            schema_version: TRACE_ARCHIVE_VERSION,
            name: "before-submit".into(),
            context_id: 7,
            pages: Vec::new(),
            task_metadata: BTreeMap::new(),
            determinism: None,
            network: NetworkArchive::default(),
            intent_cursor: 0,
            nondeterministic_regions: Vec::new(),
        })
        .expect("seal checkpoint");
        checkpoint.payload.name = "after-submit".into();
        assert!(matches!(checkpoint.verify(), Err(NetworkError::Replay(_))));
    }

    #[test]
    fn failed_task_artifact_is_content_addressed_and_tamper_evident() {
        let checkpoint = ContextCheckpoint::seal(CheckpointPayload {
            schema_version: TRACE_ARCHIVE_VERSION,
            name: "failure-root".into(),
            context_id: 9,
            pages: Vec::new(),
            task_metadata: BTreeMap::new(),
            determinism: None,
            network: NetworkArchive::default(),
            intent_cursor: 0,
            nondeterministic_regions: Vec::new(),
        })
        .expect("checkpoint");
        let trace = TaskTraceArchive {
            schema_version: TRACE_ARCHIVE_VERSION,
            context_id: 9,
            determinism: None,
            page_events: Vec::new(),
            page_traces: BTreeMap::new(),
            intents: Vec::new(),
            domain_decisions: Vec::new(),
            network: NetworkArchive::default(),
            nondeterministic_regions: Vec::new(),
        };
        let mut artifact = FailureArtifact::seal(FailureArtifactPayload {
            schema_version: TRACE_ARCHIVE_VERSION,
            failure_id: "fixture-failure".into(),
            checkpoint,
            trace,
            error: "expected fixture failure".into(),
        })
        .expect("seal failure");
        artifact.verify().expect("valid failure artifact");
        artifact.payload.error.push('!');
        assert!(matches!(artifact.verify(), Err(NetworkError::Replay(_))));
    }
}
