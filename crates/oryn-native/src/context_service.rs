#![cfg(unix)]

//! Typed parent services available to a sandboxed page worker.
//!
//! This channel deliberately has no generic filesystem, environment, shell,
//! socket, or secret operation. Networking continues to use the separately
//! policy-enforced network broker; this service coordinates context state.

use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    os::unix::net::UnixStream,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use oryn_common::v3::{PageId, PageLifecycleEvent};
use serde::{Deserialize, Serialize};
use tungstenite::{
    Message, WebSocket,
    client::IntoClientRequest,
    protocol::{CloseFrame, frame::coding::CloseCode},
    stream::MaybeTlsStream,
};
use url::Url;

use crate::{
    network::{
        NetworkBroker, NetworkRequest, SharedNetworkBroker, sanitize_parent_response_headers,
        sanitize_worker_headers,
    },
    runtime::{BrowserContext, RuntimeError},
    storage::{StorageArea, StorageChange, StorageRequest, StorageResponse, StorageScope},
};

pub const CONTEXT_SERVICE_PROTOCOL_VERSION: u32 = 3;
pub const MAX_CONTEXT_FRAME_BYTES: usize = 24 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum ContextServiceRequest {
    HttpStreamStart {
        page_id: PageId,
        request: HttpStreamRequest,
    },
    HttpStreamRead {
        page_id: PageId,
        stream_id: u64,
        max_bytes: usize,
    },
    HttpStreamAbort {
        page_id: PageId,
        stream_id: u64,
    },
    Storage {
        request: StorageRequest,
    },
    OpenPage {
        opener_page_id: PageId,
        url: Option<String>,
    },
    ActivatePage {
        source_page_id: PageId,
        target_page_id: PageId,
    },
    CommitPage {
        page_id: PageId,
        url: String,
    },
    NavigatePage {
        page_id: PageId,
        url: String,
    },
    ClosePage {
        source_page_id: PageId,
        target_page_id: PageId,
    },
    PostMessage {
        source_page_id: PageId,
        target_page_id: PageId,
        target_origin: String,
        data: serde_json::Value,
    },
    PollEvents {
        page_id: PageId,
        after_event_id: u64,
        max_events: usize,
    },
    WebSocketOpen {
        page_id: PageId,
        url: String,
        protocols: Vec<String>,
    },
    WebSocketSend {
        page_id: PageId,
        socket_id: u64,
        frame: WebSocketFrame,
    },
    WebSocketPoll {
        page_id: PageId,
        socket_id: u64,
        max_frames: usize,
    },
    WebSocketClose {
        page_id: PageId,
        socket_id: u64,
        code: Option<u16>,
        reason: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WebSocketFrame {
    Text { value: String },
    Binary { base64: String },
    Close { code: Option<u16>, reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpStreamRequest {
    pub request_id: u64,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body_base64: String,
    #[serde(default = "default_http_credentials")]
    pub credentials: bool,
}

const fn default_http_credentials() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum ContextServiceResponse {
    HttpStreamStarted {
        stream_id: u64,
        request_id: u64,
        status: u16,
        headers: Vec<(String, String)>,
        total_bytes: usize,
    },
    HttpStreamChunk {
        stream_id: u64,
        body_base64: String,
        done: bool,
    },
    HttpStreamAborted,
    Storage {
        response: StorageResponse,
    },
    PageOpened {
        page_id: PageId,
    },
    PageDenied {
        reason: String,
    },
    PageActivated {
        page_id: PageId,
    },
    PageClosed,
    PageCommitted,
    NavigationQueued,
    MessageQueued,
    Events {
        events: Vec<ContextEventRecord>,
    },
    WebSocketOpened {
        socket_id: u64,
        protocol: String,
    },
    WebSocketSent,
    WebSocketEvents {
        frames: Vec<WebSocketFrame>,
    },
    WebSocketClosed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextEventRecord {
    pub event_id: u64,
    pub event: ContextEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContextEvent {
    Storage {
        source_page_id: PageId,
        change: StorageChange,
    },
    Message {
        source_page_id: PageId,
        target_page_id: PageId,
        source_origin: String,
        data: serde_json::Value,
    },
    PageLifecycle {
        event: PageLifecycleEvent,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct RequestEnvelope {
    protocol_version: u32,
    message_id: u64,
    request: ContextServiceRequest,
}

#[derive(Debug, Serialize, Deserialize)]
struct ResponseEnvelope {
    protocol_version: u32,
    correlation_id: u64,
    response: Option<ContextServiceResponse>,
    error: Option<String>,
}

pub struct IpcContextClient {
    stream: UnixStream,
    next_message_id: u64,
}

impl IpcContextClient {
    pub fn new(stream: UnixStream) -> Self {
        Self {
            stream,
            next_message_id: 1,
        }
    }

    pub fn request(
        &mut self,
        request: ContextServiceRequest,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        let message_id = self.next_message_id;
        self.next_message_id = self.next_message_id.saturating_add(1);
        let envelope = RequestEnvelope {
            protocol_version: CONTEXT_SERVICE_PROTOCOL_VERSION,
            message_id,
            request,
        };
        let encoded = serde_json::to_vec(&envelope)
            .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
        if encoded.len() > MAX_CONTEXT_FRAME_BYTES {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "context-service IPC frame".into(),
            });
        }
        self.stream
            .write_all(&encoded)
            .and_then(|_| self.stream.write_all(b"\n"))
            .and_then(|_| self.stream.flush())
            .map_err(|error| RuntimeError::WorkerCrashed(error.to_string()))?;
        let mut reader = BufReader::new(
            self.stream
                .try_clone()
                .map_err(|error| RuntimeError::WorkerCrashed(error.to_string()))?,
        );
        let mut line = Vec::new();
        read_bounded_line(&mut reader, &mut line, MAX_CONTEXT_FRAME_BYTES)
            .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
        let response: ResponseEnvelope = serde_json::from_slice(&line)
            .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
        if response.protocol_version != CONTEXT_SERVICE_PROTOCOL_VERSION
            || response.correlation_id != message_id
        {
            return Err(RuntimeError::ProtocolViolation(
                "context-service response correlation mismatch".into(),
            ));
        }
        if let Some(error) = response.error {
            return Err(RuntimeError::ContextService(error));
        }
        response.response.ok_or_else(|| {
            RuntimeError::ProtocolViolation("context service returned no response".into())
        })
    }
}

trait ContextTransport: Send + Sync {
    fn request(
        &self,
        request: ContextServiceRequest,
    ) -> Result<ContextServiceResponse, RuntimeError>;
}

struct IpcContextTransport(Mutex<IpcContextClient>);

impl ContextTransport for IpcContextTransport {
    fn request(
        &self,
        request: ContextServiceRequest,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.0
            .lock()
            .map_err(|_| RuntimeError::ContextService("context client lock poisoned".into()))?
            .request(request)
    }
}

#[derive(Clone)]
pub struct SharedContextClient(Arc<dyn ContextTransport>);

impl SharedContextClient {
    pub fn new(client: IpcContextClient) -> Self {
        Self(Arc::new(IpcContextTransport(Mutex::new(client))))
    }

    #[cfg(feature = "v8-host")]
    pub(crate) fn direct(context: BrowserContext, source_page_id: PageId) -> Self {
        Self(Arc::new(ParentContextService::new(context, source_page_id)))
    }

    pub fn request(
        &self,
        request: ContextServiceRequest,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.0.request(request)
    }
}

pub struct ParentContextService {
    context: BrowserContext,
    source_page_id: PageId,
    next_socket_id: AtomicU64,
    sockets: Mutex<BTreeMap<u64, WebSocket<MaybeTlsStream<TcpStream>>>>,
    network: Option<SharedNetworkBroker>,
    next_http_stream_id: AtomicU64,
    http_streams: Mutex<BTreeMap<u64, HttpStreamState>>,
}

struct HttpStreamState {
    page_id: PageId,
    body: Vec<u8>,
    offset: usize,
}

impl ParentContextService {
    pub fn new(context: BrowserContext, source_page_id: PageId) -> Self {
        let network = context.shared_network_broker();
        Self::new_with_network(context, source_page_id, network)
    }

    pub fn new_with_network(
        context: BrowserContext,
        source_page_id: PageId,
        network: Option<SharedNetworkBroker>,
    ) -> Self {
        Self {
            context,
            source_page_id,
            next_socket_id: AtomicU64::new(1),
            sockets: Mutex::new(BTreeMap::new()),
            network,
            next_http_stream_id: AtomicU64::new(1),
            http_streams: Mutex::new(BTreeMap::new()),
        }
    }

    pub(crate) fn handle(
        &self,
        request: ContextServiceRequest,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        match request {
            ContextServiceRequest::HttpStreamStart { page_id, request } => {
                self.start_http_stream(page_id, request)
            }
            ContextServiceRequest::HttpStreamRead {
                page_id,
                stream_id,
                max_bytes,
            } => self.read_http_stream(page_id, stream_id, max_bytes),
            ContextServiceRequest::HttpStreamAbort { page_id, stream_id } => {
                self.abort_http_stream(page_id, stream_id)
            }
            ContextServiceRequest::Storage { request } => {
                self.validate_storage_scope(&request)?;
                let response = self
                    .context
                    .context_store()
                    .execute(request)
                    .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
                if let StorageResponse::Changed { change } = &response {
                    self.context.push_context_event(ContextEvent::Storage {
                        source_page_id: self.source_page_id,
                        change: change.clone(),
                    });
                }
                Ok(ContextServiceResponse::Storage { response })
            }
            ContextServiceRequest::OpenPage {
                opener_page_id,
                url,
            } => {
                if opener_page_id != self.source_page_id {
                    return Err(RuntimeError::ContextService(
                        "worker cannot open a popup for another page".into(),
                    ));
                }
                if !self.context.popups_allowed() {
                    return Ok(ContextServiceResponse::PageDenied {
                        reason: "popup policy denied page creation".into(),
                    });
                }
                if let Some(url) = &url {
                    self.context.validate_page_url(self.source_page_id, url)?;
                }
                let opener_origin = self.context.page_origin(opener_page_id)?;
                // Clone sessionStorage before the popup's first document can
                // execute, matching the browser's initial-copy semantics.
                let initial_url = url;
                let page = self.context.new_popup(opener_page_id, None);
                let opener_scope = StorageScope {
                    origin: opener_origin.clone(),
                    top_level_page_id: opener_page_id,
                };
                let popup_scope = StorageScope {
                    origin: opener_origin,
                    top_level_page_id: page.page_id(),
                };
                if let StorageResponse::Entries { entries } = self
                    .context
                    .context_store()
                    .execute(StorageRequest::Entries {
                        area: StorageArea::Session,
                        scope: opener_scope,
                    })
                    .map_err(|error| RuntimeError::ContextService(error.to_string()))?
                {
                    for (key, value) in entries {
                        self.context
                            .context_store()
                            .execute(StorageRequest::Set {
                                area: StorageArea::Session,
                                scope: popup_scope.clone(),
                                key,
                                value,
                            })
                            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
                    }
                }
                if let Some(url) = initial_url {
                    self.context.queue_script_navigation(page.page_id(), url)?;
                }
                Ok(ContextServiceResponse::PageOpened {
                    page_id: page.page_id(),
                })
            }
            ContextServiceRequest::ActivatePage {
                source_page_id,
                target_page_id,
            } => {
                if source_page_id != self.source_page_id {
                    return Err(RuntimeError::ContextService(
                        "worker cannot impersonate another page".into(),
                    ));
                }
                self.context.activate_page(target_page_id)?;
                Ok(ContextServiceResponse::PageActivated {
                    page_id: target_page_id,
                })
            }
            ContextServiceRequest::CommitPage { page_id, url } => {
                if page_id != self.source_page_id {
                    return Err(RuntimeError::ContextService(
                        "worker cannot commit another page".into(),
                    ));
                }
                self.context.validate_page_commit_url(page_id, &url)?;
                self.context.commit_page_url(page_id, url)?;
                Ok(ContextServiceResponse::PageCommitted)
            }
            ContextServiceRequest::NavigatePage { page_id, url } => {
                if page_id != self.source_page_id {
                    return Err(RuntimeError::ContextService(
                        "worker cannot navigate another page".into(),
                    ));
                }
                self.context.validate_page_url(page_id, &url)?;
                self.context.queue_script_navigation(page_id, url)?;
                Ok(ContextServiceResponse::NavigationQueued)
            }
            ContextServiceRequest::ClosePage {
                source_page_id,
                target_page_id,
            } => {
                if source_page_id != self.source_page_id
                    || !self
                        .context
                        .may_script_close(source_page_id, target_page_id)
                {
                    return Err(RuntimeError::ContextService(
                        "page is not permitted to close the target".into(),
                    ));
                }
                let context = self.context.clone();
                std::thread::spawn(move || {
                    if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    {
                        let _ = runtime.block_on(context.close_page(target_page_id));
                    }
                });
                Ok(ContextServiceResponse::PageClosed)
            }
            ContextServiceRequest::PostMessage {
                source_page_id,
                target_page_id,
                target_origin,
                data,
            } => {
                if source_page_id != self.source_page_id {
                    return Err(RuntimeError::ContextService(
                        "worker cannot impersonate another message source".into(),
                    ));
                }
                let source_origin = self.context.page_origin(source_page_id)?;
                let actual_target_origin = self.context.page_origin(target_page_id)?;
                if target_origin != "*" && target_origin != actual_target_origin {
                    return Err(RuntimeError::ContextService(
                        "postMessage target origin did not match".into(),
                    ));
                }
                self.context.push_context_event(ContextEvent::Message {
                    source_page_id,
                    target_page_id,
                    source_origin,
                    data,
                });
                Ok(ContextServiceResponse::MessageQueued)
            }
            ContextServiceRequest::PollEvents {
                page_id,
                after_event_id,
                max_events,
            } => {
                if page_id != self.source_page_id {
                    return Err(RuntimeError::ContextService(
                        "worker cannot poll another page's events".into(),
                    ));
                }
                Ok(ContextServiceResponse::Events {
                    events: self.context.poll_context_events(
                        page_id,
                        after_event_id,
                        max_events.min(32),
                    )?,
                })
            }
            ContextServiceRequest::WebSocketOpen {
                page_id,
                url,
                protocols,
            } => self.open_websocket(page_id, &url, &protocols),
            ContextServiceRequest::WebSocketSend {
                page_id,
                socket_id,
                frame,
            } => self.send_websocket(page_id, socket_id, frame),
            ContextServiceRequest::WebSocketPoll {
                page_id,
                socket_id,
                max_frames,
            } => self.poll_websocket(page_id, socket_id, max_frames),
            ContextServiceRequest::WebSocketClose {
                page_id,
                socket_id,
                code,
                reason,
            } => self.close_websocket(page_id, socket_id, code, reason),
        }
    }

    fn validate_socket_owner(&self, page_id: PageId) -> Result<(), RuntimeError> {
        if page_id != self.source_page_id {
            return Err(RuntimeError::ContextService(
                "worker cannot operate another page's WebSocket".into(),
            ));
        }
        Ok(())
    }

    fn start_http_stream(
        &self,
        page_id: PageId,
        request: HttpStreamRequest,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.validate_socket_owner(page_id)?;
        let body = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            request.body_base64,
        )
        .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
        if body.len() > 16 * 1024 * 1024 {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "HTTP request body".into(),
            });
        }
        let mut network = self.network.clone().ok_or_else(|| {
            RuntimeError::ContextService("HTTP streaming broker is unavailable".into())
        })?;
        let response = network
            .send(NetworkRequest {
                request_id: request.request_id,
                method: request.method,
                url: request.url,
                headers: sanitize_worker_headers(request.headers),
                body,
                credentials: request.credentials,
                resolved_addrs: Vec::new(),
            })
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
        if response.body.len() > 16 * 1024 * 1024 {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "decoded HTTP response body".into(),
            });
        }
        let total_bytes = response.body.len();
        let stream_id = self.next_http_stream_id.fetch_add(1, Ordering::Relaxed);
        let mut streams = self
            .http_streams
            .lock()
            .map_err(|_| RuntimeError::ContextService("HTTP stream lock poisoned".into()))?;
        if streams.len() >= 32 {
            return Err(RuntimeError::WorkerLimitExceeded {
                resource: "pending HTTP streams".into(),
            });
        }
        streams.insert(
            stream_id,
            HttpStreamState {
                page_id,
                body: response.body,
                offset: 0,
            },
        );
        Ok(ContextServiceResponse::HttpStreamStarted {
            stream_id,
            request_id: response.request_id,
            status: response.status,
            headers: sanitize_parent_response_headers(response.headers),
            total_bytes,
        })
    }

    fn read_http_stream(
        &self,
        page_id: PageId,
        stream_id: u64,
        max_bytes: usize,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.validate_socket_owner(page_id)?;
        let mut streams = self
            .http_streams
            .lock()
            .map_err(|_| RuntimeError::ContextService("HTTP stream lock poisoned".into()))?;
        let (chunk, done) = {
            let stream = streams
                .get_mut(&stream_id)
                .ok_or_else(|| RuntimeError::ContextService("unknown HTTP stream".into()))?;
            if stream.page_id != page_id {
                return Err(RuntimeError::ContextService(
                    "worker cannot read another page's HTTP stream".into(),
                ));
            }
            let read_bytes = max_bytes.clamp(1, 64 * 1024);
            let end = stream
                .offset
                .saturating_add(read_bytes)
                .min(stream.body.len());
            let chunk = stream.body[stream.offset..end].to_vec();
            stream.offset = end;
            (chunk, stream.offset == stream.body.len())
        };
        if done {
            streams.remove(&stream_id);
        }
        Ok(ContextServiceResponse::HttpStreamChunk {
            stream_id,
            body_base64: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, chunk),
            done,
        })
    }

    fn abort_http_stream(
        &self,
        page_id: PageId,
        stream_id: u64,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.validate_socket_owner(page_id)?;
        let removed = self
            .http_streams
            .lock()
            .map_err(|_| RuntimeError::ContextService("HTTP stream lock poisoned".into()))?
            .remove(&stream_id)
            .ok_or_else(|| RuntimeError::ContextService("unknown HTTP stream".into()))?;
        if removed.page_id != page_id {
            return Err(RuntimeError::ContextService(
                "worker cannot abort another page's HTTP stream".into(),
            ));
        }
        Ok(ContextServiceResponse::HttpStreamAborted)
    }

    fn open_websocket(
        &self,
        page_id: PageId,
        url: &str,
        protocols: &[String],
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.validate_socket_owner(page_id)?;
        let url = Url::parse(url).map_err(|error| RuntimeError::Network(error.to_string()))?;
        if !matches!(url.scheme(), "ws" | "wss") {
            return Err(RuntimeError::ContextService(
                "WebSocket URL must use ws or wss".into(),
            ));
        }
        let addresses = crate::network::resolve_and_authorize(self.context.network_policy(), &url)
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
        let timeout = self.context.request_timeout();
        let mut stream = None;
        let mut last_error = None;
        for address in addresses {
            match TcpStream::connect_timeout(&address, timeout) {
                Ok(candidate) => {
                    stream = Some(candidate);
                    break;
                }
                Err(error) => last_error = Some(error),
            }
        }
        let stream = stream.ok_or_else(|| {
            RuntimeError::Network(
                last_error
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "WebSocket resolver returned no address".into()),
            )
        })?;
        stream
            .set_read_timeout(Some(Duration::from_millis(10)))
            .and_then(|_| stream.set_write_timeout(Some(timeout)))
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
        let origin = self.context.page_origin(page_id)?;
        request.headers_mut().insert(
            "origin",
            origin
                .parse()
                .map_err(|error| RuntimeError::Network(format!("invalid Origin: {error}")))?,
        );
        if let Some(cookie) = self.context.cookie_header(&url) {
            request.headers_mut().insert(
                "cookie",
                cookie
                    .parse()
                    .map_err(|error| RuntimeError::Network(format!("invalid Cookie: {error}")))?,
            );
        }
        if !protocols.is_empty() {
            request.headers_mut().insert(
                "sec-websocket-protocol",
                protocols.join(", ").parse().map_err(|error| {
                    RuntimeError::Network(format!("invalid WebSocket protocol: {error}"))
                })?,
            );
        }
        let (socket, response) = tungstenite::client_tls_with_config(request, stream, None, None)
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
        let protocol = response
            .headers()
            .get("sec-websocket-protocol")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let socket_id = self.next_socket_id.fetch_add(1, Ordering::Relaxed);
        self.sockets
            .lock()
            .map_err(|_| RuntimeError::ContextService("WebSocket lock poisoned".into()))?
            .insert(socket_id, socket);
        Ok(ContextServiceResponse::WebSocketOpened {
            socket_id,
            protocol,
        })
    }

    fn send_websocket(
        &self,
        page_id: PageId,
        socket_id: u64,
        frame: WebSocketFrame,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.validate_socket_owner(page_id)?;
        let mut sockets = self
            .sockets
            .lock()
            .map_err(|_| RuntimeError::ContextService("WebSocket lock poisoned".into()))?;
        let socket = sockets
            .get_mut(&socket_id)
            .ok_or_else(|| RuntimeError::ContextService("unknown WebSocket".into()))?;
        let message = match frame {
            WebSocketFrame::Text { value } => Message::Text(value.into()),
            WebSocketFrame::Binary { base64 } => Message::Binary(
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, base64)
                    .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?
                    .into(),
            ),
            WebSocketFrame::Close { code, reason } => Message::Close(Some(CloseFrame {
                code: code.map(CloseCode::from).unwrap_or(CloseCode::Normal),
                reason: reason.into(),
            })),
        };
        socket
            .send(message)
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
        Ok(ContextServiceResponse::WebSocketSent)
    }

    fn poll_websocket(
        &self,
        page_id: PageId,
        socket_id: u64,
        max_frames: usize,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.validate_socket_owner(page_id)?;
        let mut sockets = self
            .sockets
            .lock()
            .map_err(|_| RuntimeError::ContextService("WebSocket lock poisoned".into()))?;
        let socket = sockets
            .get_mut(&socket_id)
            .ok_or_else(|| RuntimeError::ContextService("unknown WebSocket".into()))?;
        let mut frames = Vec::new();
        while frames.len() < max_frames.min(32) {
            match socket.read() {
                Ok(Message::Text(value)) => frames.push(WebSocketFrame::Text {
                    value: value.to_string(),
                }),
                Ok(Message::Binary(value)) => frames.push(WebSocketFrame::Binary {
                    base64: base64::Engine::encode(
                        &base64::engine::general_purpose::STANDARD,
                        value,
                    ),
                }),
                Ok(Message::Close(frame)) => {
                    frames.push(WebSocketFrame::Close {
                        code: frame.as_ref().map(|frame| u16::from(frame.code)),
                        reason: frame
                            .map(|frame| frame.reason.to_string())
                            .unwrap_or_default(),
                    });
                    break;
                }
                Ok(Message::Ping(value)) => {
                    socket
                        .send(Message::Pong(value))
                        .map_err(|error| RuntimeError::Network(error.to_string()))?;
                }
                Ok(Message::Pong(_) | Message::Frame(_)) => {}
                Err(tungstenite::Error::Io(error))
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    break;
                }
                Err(tungstenite::Error::Io(error))
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::ConnectionReset
                            | std::io::ErrorKind::BrokenPipe
                            | std::io::ErrorKind::UnexpectedEof
                    ) =>
                {
                    frames.push(WebSocketFrame::Close {
                        code: None,
                        reason: String::new(),
                    });
                    break;
                }
                Err(
                    tungstenite::Error::ConnectionClosed
                    | tungstenite::Error::AlreadyClosed
                    | tungstenite::Error::Protocol(
                        tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
                    ),
                ) => {
                    frames.push(WebSocketFrame::Close {
                        code: None,
                        reason: String::new(),
                    });
                    break;
                }
                Err(error) => return Err(RuntimeError::Network(error.to_string())),
            }
        }
        Ok(ContextServiceResponse::WebSocketEvents { frames })
    }

    fn close_websocket(
        &self,
        page_id: PageId,
        socket_id: u64,
        code: Option<u16>,
        reason: String,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.validate_socket_owner(page_id)?;
        let mut socket = self
            .sockets
            .lock()
            .map_err(|_| RuntimeError::ContextService("WebSocket lock poisoned".into()))?
            .remove(&socket_id)
            .ok_or_else(|| RuntimeError::ContextService("unknown WebSocket".into()))?;
        let close = socket.close(Some(CloseFrame {
            code: code.map(CloseCode::from).unwrap_or(CloseCode::Normal),
            reason: reason.into(),
        }));
        if let Err(error) = close
            && !matches!(
                error,
                tungstenite::Error::ConnectionClosed
                    | tungstenite::Error::AlreadyClosed
                    | tungstenite::Error::Protocol(
                        tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
                    )
            )
        {
            return Err(RuntimeError::Network(error.to_string()));
        }
        Ok(ContextServiceResponse::WebSocketClosed)
    }

    fn validate_storage_scope(&self, request: &StorageRequest) -> Result<(), RuntimeError> {
        let (scope, area) = match request {
            StorageRequest::Get { area, scope, .. }
            | StorageRequest::Set { area, scope, .. }
            | StorageRequest::Remove { area, scope, .. }
            | StorageRequest::Clear { area, scope }
            | StorageRequest::Entries { area, scope } => (Some(scope), Some(area)),
            StorageRequest::IndexedDb { origin, .. } => {
                if &self.context.page_origin(self.source_page_id)? != origin {
                    return Err(RuntimeError::ContextService(
                        "IndexedDB origin did not match the worker page".into(),
                    ));
                }
                (None, None)
            }
        };
        if let (Some(scope), Some(area)) = (scope, area) {
            if scope.top_level_page_id != self.source_page_id
                || self.context.page_origin(self.source_page_id)? != scope.origin
            {
                return Err(RuntimeError::ContextService(
                    "storage scope did not match the worker page".into(),
                ));
            }
            if *area == StorageArea::Session && scope.top_level_page_id != self.source_page_id {
                return Err(RuntimeError::ContextService(
                    "session storage is scoped to the top-level page".into(),
                ));
            }
        }
        Ok(())
    }
}

impl ContextTransport for ParentContextService {
    fn request(
        &self,
        request: ContextServiceRequest,
    ) -> Result<ContextServiceResponse, RuntimeError> {
        self.handle(request)
    }
}

pub fn serve_context_service(
    stream: UnixStream,
    service: ParentContextService,
) -> Result<(), RuntimeError> {
    let reader_stream = stream
        .try_clone()
        .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
    let mut reader = BufReader::new(reader_stream);
    let mut writer = stream;
    let mut last_message_id = 0_u64;
    loop {
        let mut line = Vec::new();
        let read = read_bounded_line(&mut reader, &mut line, MAX_CONTEXT_FRAME_BYTES)
            .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
        if read == 0 {
            return Ok(());
        }
        let request = serde_json::from_slice::<RequestEnvelope>(&line);
        let (message_id, response, error) = match request {
            Ok(envelope)
                if envelope.protocol_version == CONTEXT_SERVICE_PROTOCOL_VERSION
                    && envelope.message_id > last_message_id =>
            {
                last_message_id = envelope.message_id;
                match service.handle(envelope.request) {
                    Ok(response) => (envelope.message_id, Some(response), None),
                    Err(error) => (envelope.message_id, None, Some(error.to_string())),
                }
            }
            Ok(envelope) => (
                envelope.message_id,
                None,
                Some("duplicate message id or unsupported context protocol".into()),
            ),
            Err(error) => (0, None, Some(format!("malformed context request: {error}"))),
        };
        serde_json::to_writer(
            &mut writer,
            &ResponseEnvelope {
                protocol_version: CONTEXT_SERVICE_PROTOCOL_VERSION,
                correlation_id: message_id,
                response,
                error,
            },
        )
        .map_err(|error| RuntimeError::ProtocolViolation(error.to_string()))?;
        writer
            .write_all(b"\n")
            .and_then(|_| writer.flush())
            .map_err(|error| RuntimeError::ContextService(error.to_string()))?;
    }
}

fn read_bounded_line(
    reader: &mut impl BufRead,
    output: &mut Vec<u8>,
    limit: usize,
) -> std::io::Result<usize> {
    let mut limited = reader.take(limit.saturating_add(1) as u64);
    let read = limited.read_until(b'\n', output)?;
    if output.len() > limit {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "context-service frame exceeded limit",
        ));
    }
    Ok(read)
}

#[cfg(all(test, feature = "in-process-probe"))]
mod tests {
    use super::*;
    use crate::{Browser, network::NetworkError};

    struct FixtureNetwork;

    impl NetworkBroker for FixtureNetwork {
        fn send(
            &mut self,
            request: NetworkRequest,
        ) -> Result<crate::network::NetworkResponse, NetworkError> {
            assert_eq!(
                request.headers,
                vec![("Accept".into(), "text/plain".into())]
            );
            Ok(crate::network::NetworkResponse {
                request_id: request.request_id,
                status: 200,
                headers: vec![
                    ("Content-Type".into(), "application/octet-stream".into()),
                    ("Set-Cookie".into(), "secret=yes".into()),
                ],
                body: vec![7; 150_000],
            })
        }
    }

    #[test]
    fn typed_http_streams_are_owned_bounded_and_header_filtered() {
        let context = Browser::in_process_probe().new_context();
        let page_id = PageId(91);
        let service = ParentContextService::new_with_network(
            context,
            page_id,
            Some(SharedNetworkBroker::new(FixtureNetwork)),
        );
        let started = service
            .handle(ContextServiceRequest::HttpStreamStart {
                page_id,
                request: HttpStreamRequest {
                    request_id: 7,
                    method: "GET".into(),
                    url: "https://example.test/stream".into(),
                    headers: vec![
                        ("Accept".into(), "text/plain".into()),
                        ("Authorization".into(), "secret".into()),
                    ],
                    body_base64: String::new(),
                    credentials: true,
                },
            })
            .expect("start stream");
        let ContextServiceResponse::HttpStreamStarted {
            stream_id,
            headers,
            total_bytes,
            ..
        } = started
        else {
            panic!("unexpected stream response");
        };
        assert_eq!(total_bytes, 150_000);
        assert_eq!(
            headers,
            vec![("Content-Type".into(), "application/octet-stream".into())]
        );

        let mut bytes = 0;
        let mut chunks = 0;
        loop {
            let response = service
                .handle(ContextServiceRequest::HttpStreamRead {
                    page_id,
                    stream_id,
                    max_bytes: usize::MAX,
                })
                .expect("read stream");
            let ContextServiceResponse::HttpStreamChunk {
                body_base64, done, ..
            } = response
            else {
                panic!("unexpected chunk response");
            };
            let chunk =
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, body_base64)
                    .expect("valid chunk");
            assert!(chunk.len() <= 64 * 1024);
            bytes += chunk.len();
            chunks += 1;
            if done {
                break;
            }
        }
        assert_eq!(bytes, 150_000);
        assert_eq!(chunks, 3);
        assert!(matches!(
            service.handle(ContextServiceRequest::HttpStreamRead {
                page_id: PageId(92),
                stream_id,
                max_bytes: 1,
            }),
            Err(RuntimeError::ContextService(_))
        ));
    }
}
