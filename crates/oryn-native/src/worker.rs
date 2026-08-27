use std::{
    io::{BufRead, Write},
    time::Duration,
};

use oryn_common::v2::{Observation, ObservationDelta, ProjectionProfile, Revision, SemanticRef};
use serde::{Deserialize, Serialize};

use crate::{
    Browser, BrowserConfig, ContentKind, ContextOptions, NavigationResult, PageHandle,
    WaitPredicate,
    network::SharedNetworkBroker,
    oil::{NativeOilError, NativeOilOutput, NativeOilSession},
    runtime::NativeAction,
};

pub const WORKER_PROTOCOL_VERSION: u32 = 2;
pub const MAX_IPC_FRAME_BYTES: usize = 24 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerLimits {
    pub v8_heap_bytes: usize,
    pub worker_rss_bytes: usize,
    pub ipc_frame_bytes: usize,
    pub command_wall_ms: u64,
    pub navigation_wall_ms: u64,
    pub queue_capacity: usize,
}

impl Default for WorkerLimits {
    fn default() -> Self {
        Self::from(&BrowserConfig::default())
    }
}

impl From<&BrowserConfig> for WorkerLimits {
    fn from(config: &BrowserConfig) -> Self {
        Self {
            v8_heap_bytes: config.max_v8_heap_bytes,
            worker_rss_bytes: config.max_worker_rss_bytes,
            ipc_frame_bytes: config.max_ipc_frame_bytes,
            command_wall_ms: config.command_timeout.as_millis() as u64,
            navigation_wall_ms: config.navigation_timeout.as_millis() as u64,
            queue_capacity: config.worker_queue_capacity,
        }
    }
}

impl WorkerLimits {
    pub fn browser_config(&self) -> BrowserConfig {
        BrowserConfig {
            worker_queue_capacity: self.queue_capacity.min(32),
            request_timeout: Duration::from_millis(self.navigation_wall_ms.min(30_000)),
            max_response_bytes: 16 * 1024 * 1024,
            max_redirects: 10,
            max_v8_heap_bytes: self.v8_heap_bytes.min(256 * 1024 * 1024),
            max_worker_rss_bytes: self.worker_rss_bytes.min(512 * 1024 * 1024),
            max_ipc_frame_bytes: self.ipc_frame_bytes.min(MAX_IPC_FRAME_BYTES),
            command_timeout: Duration::from_millis(self.command_wall_ms.min(10_000)),
            navigation_timeout: Duration::from_millis(self.navigation_wall_ms.min(30_000)),
            document_generation_base: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostEnvelope {
    pub protocol_version: u32,
    pub session_id: u64,
    pub message_id: u64,
    pub command: WorkerRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerEnvelope {
    pub protocol_version: u32,
    pub session_id: u64,
    pub message_id: u64,
    pub correlation_id: u64,
    pub response: WorkerResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum WorkerRequest {
    Initialize {
        nonce: String,
        limits: WorkerLimits,
        context: ContextOptions,
        document_generation_base: u64,
    },
    Ping,
    Goto {
        url: String,
    },
    Back,
    Forward,
    Refresh,
    LoadHtml {
        url: String,
        html: String,
    },
    WaitFor {
        predicate: WaitPredicate,
    },
    Evaluate {
        source: String,
    },
    Capabilities,
    ResolveSelector {
        selector: String,
        action: oryn_common::v2::SemanticAction,
    },
    Extract {
        kind: ContentKind,
        selector: Option<String>,
    },
    Observe {
        profile: Option<ProjectionProfile>,
    },
    ObserveDelta {
        from: Revision,
    },
    Execute {
        action: NativeActionRequest,
    },
    Oil {
        input: String,
    },
    Trace,
    Close,
    Exit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeActionRequest {
    pub target: SemanticRef,
    pub action: oryn_common::v2::SemanticAction,
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum WorkerResponse {
    Ready {
        nonce: String,
        protocol_version: u32,
        runtime: Box<crate::RuntimeBuildInfo>,
        binary_sha256: String,
        policy_digest: String,
        sandbox_state: String,
        secret_environment_present: bool,
        limits: WorkerLimits,
    },
    Pong {
        protocol_version: u32,
    },
    Loaded {
        revision: Revision,
    },
    Navigated {
        navigation: NavigationResult,
    },
    Observation {
        observation: Observation,
    },
    Delta {
        delta: ObservationDelta,
    },
    Action {
        result: oryn_common::v2::ActionResult,
    },
    Oil {
        outputs: Vec<NativeOilOutput>,
    },
    Trace {
        events: Vec<crate::trace::TraceEvent>,
    },
    Capabilities {
        capabilities: Vec<oryn_common::v2::CapabilityDiagnostic>,
    },
    SemanticRef {
        semantic_ref: SemanticRef,
    },
    Structured {
        value: serde_json::Value,
    },
    Value {
        value: String,
    },
    Satisfied,
    Closed,
    Exiting,
    Error {
        error: Box<crate::runtime::RuntimeError>,
    },
    LimitExceeded {
        resource: String,
    },
}

struct PageSession {
    page: PageHandle,
    oil: NativeOilSession,
}

impl PageSession {
    fn new(
        config: BrowserConfig,
        context: ContextOptions,
        broker: Option<SharedNetworkBroker>,
    ) -> Self {
        let browser = Browser::in_process_with_config(config);
        let page = match broker {
            Some(broker) => browser.new_context_with_broker(context, broker).new_page(),
            None => browser.new_context_with_options(context).new_page(),
        };
        let oil = NativeOilSession::new(page.clone());
        Self { page, oil }
    }
}

pub async fn serve<R: BufRead, W: Write>(reader: R, writer: W) -> std::io::Result<()> {
    serve_with_broker(reader, writer, None).await
}

pub async fn serve_with_broker<R: BufRead, W: Write>(
    mut reader: R,
    mut writer: W,
    broker: Option<SharedNetworkBroker>,
) -> std::io::Result<()> {
    let mut session: Option<PageSession> = None;
    let mut active_session_id: Option<u64> = None;
    let mut last_message_id = 0_u64;
    loop {
        let mut line = Vec::new();
        let mut limited =
            std::io::Read::take(&mut reader, MAX_IPC_FRAME_BYTES.saturating_add(1) as u64);
        let read = limited.read_until(b'\n', &mut line)?;
        if read == 0 {
            break;
        }
        if line.len() > MAX_IPC_FRAME_BYTES {
            write_legacy(
                &mut writer,
                &WorkerResponse::Error {
                    error: Box::new(crate::runtime::RuntimeError::WorkerLimitExceeded {
                        resource: "IPC frame".into(),
                    }),
                },
            )?;
            break;
        }

        if let Ok(envelope) = serde_json::from_slice::<HostEnvelope>(&line) {
            let response = if envelope.protocol_version != WORKER_PROTOCOL_VERSION {
                WorkerResponse::Error {
                    error: Box::new(crate::runtime::RuntimeError::ProtocolViolation(
                        "unsupported worker protocol version".into(),
                    )),
                }
            } else if active_session_id.is_some_and(|session_id| session_id != envelope.session_id)
            {
                WorkerResponse::Error {
                    error: Box::new(crate::runtime::RuntimeError::ProtocolViolation(
                        "worker session identifier changed".into(),
                    )),
                }
            } else if envelope.message_id <= last_message_id {
                WorkerResponse::Error {
                    error: Box::new(crate::runtime::RuntimeError::ProtocolViolation(
                        "duplicate or out-of-order worker message".into(),
                    )),
                }
            } else if active_session_id.is_some()
                && matches!(&envelope.command, WorkerRequest::Initialize { .. })
            {
                WorkerResponse::Error {
                    error: Box::new(crate::runtime::RuntimeError::ProtocolViolation(
                        "worker session is already initialized".into(),
                    )),
                }
            } else {
                last_message_id = envelope.message_id;
                if matches!(&envelope.command, WorkerRequest::Initialize { .. }) {
                    active_session_id = Some(envelope.session_id);
                }
                dispatch(&mut session, broker.clone(), envelope.command).await
            };
            let exiting = matches!(response, WorkerResponse::Exiting);
            let response = WorkerEnvelope {
                protocol_version: WORKER_PROTOCOL_VERSION,
                session_id: envelope.session_id,
                message_id: envelope.message_id,
                correlation_id: envelope.message_id,
                response,
            };
            serde_json::to_writer(&mut writer, &response)?;
            writer.write_all(b"\n")?;
            writer.flush()?;
            if exiting {
                break;
            }
            continue;
        }

        // Protocol-v1 compatibility is intentionally limited to diagnostics.
        let response = match serde_json::from_slice::<WorkerRequest>(&line) {
            Ok(WorkerRequest::Ping) => WorkerResponse::Pong {
                protocol_version: WORKER_PROTOCOL_VERSION,
            },
            Ok(WorkerRequest::Exit) => WorkerResponse::Exiting,
            Ok(_) => WorkerResponse::Error {
                error: Box::new(crate::runtime::RuntimeError::ProtocolViolation(
                    "page commands require worker protocol v2 initialization".into(),
                )),
            },
            Err(error) => WorkerResponse::Error {
                error: Box::new(crate::runtime::RuntimeError::ProtocolViolation(format!(
                    "invalid worker request: {error}"
                ))),
            },
        };
        let exiting = matches!(response, WorkerResponse::Exiting);
        write_legacy(&mut writer, &response)?;
        if exiting {
            break;
        }
    }
    if let Some(session) = session {
        let _ = session.page.close().await;
    }
    Ok(())
}

fn write_legacy(writer: &mut impl Write, response: &WorkerResponse) -> std::io::Result<()> {
    serde_json::to_writer(&mut *writer, response)?;
    writer.write_all(b"\n")?;
    writer.flush()
}

async fn dispatch(
    session: &mut Option<PageSession>,
    broker: Option<SharedNetworkBroker>,
    request: WorkerRequest,
) -> WorkerResponse {
    if let WorkerRequest::Initialize {
        nonce,
        limits,
        context,
        document_generation_base,
    } = request
    {
        let policy_digest = policy_digest(&context);
        let effective = WorkerLimits::from(&limits.browser_config());
        let mut config = effective.browser_config();
        config.document_generation_base = document_generation_base;
        *session = Some(PageSession::new(config, context, broker));
        return WorkerResponse::Ready {
            nonce,
            protocol_version: WORKER_PROTOCOL_VERSION,
            runtime: Box::new(crate::runtime_build_info()),
            binary_sha256: current_binary_sha256().unwrap_or_default(),
            policy_digest,
            sandbox_state: if cfg!(target_os = "macos") {
                "app_sandbox".into()
            } else {
                "process_isolated_unsandboxed".into()
            },
            secret_environment_present: secret_environment_present(),
            limits: effective,
        };
    }
    if matches!(request, WorkerRequest::Ping) {
        return WorkerResponse::Pong {
            protocol_version: WORKER_PROTOCOL_VERSION,
        };
    }
    if matches!(request, WorkerRequest::Exit) {
        return WorkerResponse::Exiting;
    }
    let Some(session) = session.as_mut() else {
        return WorkerResponse::Error {
            error: Box::new(crate::runtime::RuntimeError::ProtocolViolation(
                "worker has not been initialized".into(),
            )),
        };
    };
    match request {
        WorkerRequest::Goto { url } => navigation(session.page.goto(url).await),
        WorkerRequest::Back => navigation(session.page.back().await),
        WorkerRequest::Forward => navigation(session.page.forward().await),
        WorkerRequest::Refresh => navigation(session.page.refresh().await),
        WorkerRequest::LoadHtml { url, html } => match session.page.load_html(url, html).await {
            Ok(revision) => WorkerResponse::Loaded { revision },
            Err(value) => error(value),
        },
        WorkerRequest::WaitFor { predicate } => match session.page.wait_for(predicate).await {
            Ok(()) => WorkerResponse::Satisfied,
            Err(value) => error(value),
        },
        WorkerRequest::Evaluate { source } => match session.page.evaluate(source).await {
            Ok(value) => WorkerResponse::Value { value },
            Err(value) => error(value),
        },
        WorkerRequest::Capabilities => match session.page.capabilities().await {
            Ok(capabilities) => WorkerResponse::Capabilities { capabilities },
            Err(value) => error(value),
        },
        WorkerRequest::ResolveSelector { selector, action } => {
            match session.page.resolve_selector(selector, action).await {
                Ok(semantic_ref) => WorkerResponse::SemanticRef { semantic_ref },
                Err(value) => error(value),
            }
        }
        WorkerRequest::Extract { kind, selector } => {
            match session.page.extract(kind, selector).await {
                Ok(value) => WorkerResponse::Structured { value },
                Err(value) => error(value),
            }
        }
        WorkerRequest::Observe { profile } => {
            match session
                .page
                .observe_profile(profile.unwrap_or(ProjectionProfile::Compact))
                .await
            {
                Ok(observation) => WorkerResponse::Observation { observation },
                Err(value) => error(value),
            }
        }
        WorkerRequest::ObserveDelta { from } => match session.page.observe_delta(from).await {
            Ok(delta) => WorkerResponse::Delta { delta },
            Err(value) => error(value),
        },
        WorkerRequest::Execute { action } => match session
            .page
            .execute(NativeAction {
                target: action.target,
                action: action.action,
                value: action.value,
            })
            .await
        {
            Ok(result) => WorkerResponse::Action { result },
            Err(value) => error(value),
        },
        WorkerRequest::Oil { input } => match session.oil.execute(&input).await {
            Ok(outputs) => WorkerResponse::Oil { outputs },
            Err(NativeOilError::Runtime(value)) => error(value),
            Err(value) => error(crate::runtime::RuntimeError::Script(value.to_string())),
        },
        WorkerRequest::Trace => match session.page.trace().await {
            Ok(events) => WorkerResponse::Trace { events },
            Err(value) => error(value),
        },
        WorkerRequest::Close => match session.page.close().await {
            Ok(()) => WorkerResponse::Closed,
            Err(value) => error(value),
        },
        WorkerRequest::Initialize { .. } | WorkerRequest::Ping | WorkerRequest::Exit => {
            unreachable!("handled before session dispatch")
        }
    }
}

fn navigation(result: Result<NavigationResult, crate::runtime::RuntimeError>) -> WorkerResponse {
    match result {
        Ok(navigation) => WorkerResponse::Navigated { navigation },
        Err(value) => error(value),
    }
}

fn error(value: crate::runtime::RuntimeError) -> WorkerResponse {
    match value {
        crate::runtime::RuntimeError::WorkerLimitExceeded { resource } => {
            WorkerResponse::LimitExceeded { resource }
        }
        value => WorkerResponse::Error {
            error: Box::new(value),
        },
    }
}

fn current_binary_sha256() -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(std::env::current_exe()?)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub fn policy_digest(context: &ContextOptions) -> String {
    use sha2::{Digest, Sha256};
    let encoded = serde_json::to_vec(&context.network_policy)
        .expect("network policy serialization is infallible");
    format!("{:x}", Sha256::digest(encoded))
}

fn secret_environment_present() -> bool {
    [
        "AZURE_OPENAI_API_KEY",
        "OPENAI_API_KEY",
        "OLLAMA_API_KEY",
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "ALL_PROXY",
        "HOME",
        "DYLD_LIBRARY_PATH",
        "DYLD_INSERT_LIBRARIES",
    ]
    .iter()
    .any(|name| std::env::var_os(name).is_some())
}

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Cursor};

    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn serves_initialized_page_requests_over_v2_json_lines() {
        let requests = vec![
            WorkerRequest::Initialize {
                nonce: "test-nonce".into(),
                limits: WorkerLimits::default(),
                context: ContextOptions::default(),
                document_generation_base: 0,
            },
            WorkerRequest::LoadHtml {
                url: "https://example.test".into(),
                html: "<button>Save</button>".into(),
            },
            WorkerRequest::Observe { profile: None },
            WorkerRequest::Oil {
                input: "observe\nclick \"Save\"".into(),
            },
            WorkerRequest::Trace,
            WorkerRequest::Exit,
        ];
        let mut input = Vec::new();
        for (index, command) in requests.into_iter().enumerate() {
            serde_json::to_writer(
                &mut input,
                &HostEnvelope {
                    protocol_version: WORKER_PROTOCOL_VERSION,
                    session_id: 7,
                    message_id: index as u64 + 1,
                    command,
                },
            )
            .expect("serialize request");
            input.push(b'\n');
        }
        let mut output = Vec::new();
        serve(BufReader::new(Cursor::new(input)), &mut output)
            .await
            .expect("serve requests");
        let lines: Vec<WorkerEnvelope> = output
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).expect("valid response"))
            .collect();

        assert_eq!(lines.len(), 6);
        assert!(matches!(lines[0].response, WorkerResponse::Ready { .. }));
        assert!(matches!(lines[1].response, WorkerResponse::Loaded { .. }));
        assert!(matches!(
            lines[2].response,
            WorkerResponse::Observation { .. }
        ));
        assert!(matches!(lines[3].response, WorkerResponse::Oil { .. }));
        assert!(matches!(lines[4].response, WorkerResponse::Trace { .. }));
        assert!(matches!(lines[5].response, WorkerResponse::Exiting));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn legacy_protocol_only_allows_ping_and_exit() {
        let input = br#"{"command":"ping"}
{"command":"observe","profile":null}
{"command":"exit"}
"#;
        let mut output = Vec::new();
        serve(BufReader::new(Cursor::new(input)), &mut output)
            .await
            .expect("serve legacy requests");
        let values = output
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(values[0]["status"], "pong");
        assert_eq!(values[1]["status"], "error");
        assert_eq!(values[2]["status"], "exiting");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn rejects_duplicate_unmatched_and_reinitialized_v2_messages() {
        let commands = [
            (
                9,
                1,
                WorkerRequest::Initialize {
                    nonce: "nonce".into(),
                    limits: WorkerLimits::default(),
                    context: ContextOptions::default(),
                    document_generation_base: 0,
                },
            ),
            (9, 1, WorkerRequest::Ping),
            (10, 2, WorkerRequest::Ping),
            (
                9,
                3,
                WorkerRequest::Initialize {
                    nonce: "again".into(),
                    limits: WorkerLimits::default(),
                    context: ContextOptions::default(),
                    document_generation_base: 0,
                },
            ),
            (9, 4, WorkerRequest::Exit),
        ];
        let mut input = Vec::new();
        for (session_id, message_id, command) in commands {
            serde_json::to_writer(
                &mut input,
                &HostEnvelope {
                    protocol_version: WORKER_PROTOCOL_VERSION,
                    session_id,
                    message_id,
                    command,
                },
            )
            .unwrap();
            input.push(b'\n');
        }
        let mut output = Vec::new();
        serve(BufReader::new(Cursor::new(input)), &mut output)
            .await
            .unwrap();
        let responses = output
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice::<WorkerEnvelope>(line).unwrap())
            .collect::<Vec<_>>();
        assert!(matches!(
            responses[0].response,
            WorkerResponse::Ready { .. }
        ));
        assert!(matches!(
            responses[1].response,
            WorkerResponse::Error { .. }
        ));
        assert!(matches!(
            responses[2].response,
            WorkerResponse::Error { .. }
        ));
        assert!(matches!(
            responses[3].response,
            WorkerResponse::Error { .. }
        ));
        assert!(matches!(responses[4].response, WorkerResponse::Exiting));
    }
}
