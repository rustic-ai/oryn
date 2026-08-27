use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    os::unix::net::UnixStream,
    sync::{Arc, Mutex},
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::security::{NetworkPolicy, PolicyDenied};

const BROKER_PROTOCOL_VERSION: u32 = 2;
const MAX_BROKER_FRAME_BYTES: usize = 24 * 1024 * 1024;
const MAX_DECODED_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkRequest {
    pub request_id: u64,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    #[serde(default, skip_serializing, skip_deserializing)]
    pub resolved_addrs: Vec<SocketAddr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkResponse {
    pub request_id: u64,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

pub trait NetworkBroker: Send {
    fn send(&mut self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError>;
}

#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
    #[error(transparent)]
    Policy(#[from] PolicyDenied),
    #[error("no host network transport is configured")]
    Unavailable,
    #[error("host resolution failed for {host}: {detail}")]
    Resolution { host: String, detail: String },
    #[error("network transport failed: {0}")]
    Transport(String),
    #[error("response exceeded the configured {limit}-byte limit")]
    ResponseTooLarge { limit: usize },
}

pub struct PolicyBroker<B> {
    policy: NetworkPolicy,
    inner: B,
}

pub struct SharedNetworkBroker {
    inner: Option<Arc<Mutex<Box<dyn NetworkBroker>>>>,
}

impl SharedNetworkBroker {
    pub fn new(broker: impl NetworkBroker + 'static) -> Self {
        Self {
            inner: Some(Arc::new(Mutex::new(Box::new(broker)))),
        }
    }

    pub fn lock(&self) -> std::sync::LockResult<std::sync::MutexGuard<'_, Box<dyn NetworkBroker>>> {
        self.inner.as_ref().expect("broker handle is live").lock()
    }
}

impl Clone for SharedNetworkBroker {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl Drop for SharedNetworkBroker {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.take() {
            // reqwest's blocking client owns a Tokio runtime and must not be
            // destroyed from within an async executor thread.
            std::thread::spawn(move || drop(inner));
        }
    }
}

impl<B> PolicyBroker<B> {
    pub fn new(policy: NetworkPolicy, inner: B) -> Self {
        Self { policy, inner }
    }

    fn authorize(&self, request: &NetworkRequest) -> Result<Vec<SocketAddr>, NetworkError> {
        let url = Url::parse(&request.url)
            .map_err(|error| NetworkError::InvalidUrl(error.to_string()))?;
        self.policy.check_scheme(url.scheme())?;
        let mut vetted = Vec::new();
        if let Some(host) = url.host_str() {
            if (host.eq_ignore_ascii_case("localhost") || host.ends_with(".localhost"))
                && !self.policy.allow_loopback
            {
                return Err(NetworkError::Policy(PolicyDenied::Loopback));
            }
            if let Ok(address) = host.parse::<IpAddr>() {
                self.policy.check_ip(address)?;
                let port = url.port_or_known_default().ok_or_else(|| {
                    NetworkError::InvalidUrl("URL has no explicit or scheme-default port".into())
                })?;
                vetted.push(SocketAddr::new(address, port));
            } else {
                let port = url.port_or_known_default().ok_or_else(|| {
                    NetworkError::InvalidUrl("URL has no explicit or scheme-default port".into())
                })?;
                let addresses =
                    (host, port)
                        .to_socket_addrs()
                        .map_err(|error| NetworkError::Resolution {
                            host: host.to_string(),
                            detail: error.to_string(),
                        })?;
                let mut resolved = false;
                for address in addresses {
                    resolved = true;
                    self.policy.check_ip(address.ip())?;
                    vetted.push(address);
                }
                if !resolved {
                    return Err(NetworkError::Resolution {
                        host: host.to_string(),
                        detail: "resolver returned no addresses".into(),
                    });
                }
            }
        }
        Ok(vetted)
    }
}

/// A single-request rustls transport. Redirects are intentionally disabled so
/// callers can send every hop back through `PolicyBroker`.
pub struct RustlsTransport {
    timeout: Duration,
    cookies: Arc<reqwest::cookie::Jar>,
    max_response_bytes: usize,
}

impl RustlsTransport {
    pub fn new(timeout: Duration, max_response_bytes: usize) -> Result<Self, NetworkError> {
        Self::with_cookie_jar(
            timeout,
            max_response_bytes,
            Arc::new(reqwest::cookie::Jar::default()),
        )
    }

    pub fn with_cookie_jar(
        timeout: Duration,
        max_response_bytes: usize,
        cookies: Arc<reqwest::cookie::Jar>,
    ) -> Result<Self, NetworkError> {
        Ok(Self {
            timeout,
            cookies,
            max_response_bytes,
        })
    }
}

impl NetworkBroker for RustlsTransport {
    fn send(&mut self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        let url = Url::parse(&request.url)
            .map_err(|error| NetworkError::InvalidUrl(error.to_string()))?;
        let mut client = reqwest::blocking::Client::builder()
            .timeout(self.timeout)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .cookie_provider(self.cookies.clone());
        if let Some(host) = url.host_str()
            && !request.resolved_addrs.is_empty()
        {
            client = client.resolve_to_addrs(host, &request.resolved_addrs);
        }
        let client = client
            .build()
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        let mut builder = client.request(method, url);
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        if !request.body.is_empty() {
            builder = builder.body(request.body);
        }
        let mut response = builder
            .send()
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        if response
            .content_length()
            .is_some_and(|length| length > self.max_response_bytes as u64)
        {
            return Err(NetworkError::ResponseTooLarge {
                limit: self.max_response_bytes,
            });
        }
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_string(),
                    value.to_str().unwrap_or_default().to_string(),
                )
            })
            .collect();
        let mut body = Vec::new();
        response
            .by_ref()
            .take(self.max_response_bytes as u64 + 1)
            .read_to_end(&mut body)
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        if body.len() > self.max_response_bytes {
            return Err(NetworkError::ResponseTooLarge {
                limit: self.max_response_bytes,
            });
        }
        Ok(NetworkResponse {
            request_id: request.request_id,
            status,
            headers,
            body,
        })
    }
}

/// The only privileged service exposed to a page worker. The connected Unix
/// socket is inherited from the parent, so the sandboxed child needs no
/// filesystem or network entitlement of its own.
pub struct IpcNetworkBroker {
    stream: UnixStream,
    next_message_id: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct BrokerRequestEnvelope {
    protocol_version: u32,
    message_id: u64,
    request: EncodedNetworkRequest,
}

#[derive(Debug, Serialize, Deserialize)]
struct BrokerResponseEnvelope {
    protocol_version: u32,
    correlation_id: u64,
    response: Option<EncodedNetworkResponse>,
    error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct EncodedNetworkRequest {
    request_id: u64,
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body_base64: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct EncodedNetworkResponse {
    request_id: u64,
    status: u16,
    headers: Vec<(String, String)>,
    body_base64: String,
}

impl IpcNetworkBroker {
    pub fn new(stream: UnixStream) -> Self {
        Self {
            stream,
            next_message_id: 1,
        }
    }
}

impl NetworkBroker for IpcNetworkBroker {
    fn send(&mut self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        let message_id = self.next_message_id;
        self.next_message_id += 1;
        let request_id = request.request_id;
        let envelope = BrokerRequestEnvelope {
            protocol_version: BROKER_PROTOCOL_VERSION,
            message_id,
            request: EncodedNetworkRequest {
                request_id: request.request_id,
                method: request.method,
                url: request.url,
                headers: request.headers,
                body_base64: BASE64.encode(request.body),
            },
        };
        serde_json::to_writer(&mut self.stream, &envelope)
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        self.stream
            .write_all(b"\n")
            .and_then(|_| self.stream.flush())
            .map_err(|error| NetworkError::Transport(error.to_string()))?;

        let mut line = Vec::new();
        let mut reader = BufReader::new(
            self.stream
                .try_clone()
                .map_err(|error| NetworkError::Transport(error.to_string()))?,
        );
        read_bounded_line(&mut reader, &mut line, MAX_BROKER_FRAME_BYTES)
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        if line.len() > MAX_BROKER_FRAME_BYTES {
            return Err(NetworkError::Transport(
                "broker response exceeded the IPC frame limit".into(),
            ));
        }
        let response: BrokerResponseEnvelope = serde_json::from_slice(&line)
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        if response.protocol_version != BROKER_PROTOCOL_VERSION
            || response.correlation_id != message_id
        {
            return Err(NetworkError::Transport(
                "broker response correlation or protocol mismatch".into(),
            ));
        }
        if let Some(error) = response.error {
            return Err(NetworkError::Transport(error));
        }
        let response = response
            .response
            .ok_or_else(|| NetworkError::Transport("broker returned no response".into()))?;
        if response.request_id != request_id {
            return Err(NetworkError::Transport(
                "broker response request identifier mismatch".into(),
            ));
        }
        let body = BASE64
            .decode(response.body_base64)
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        if body.len() > MAX_DECODED_RESPONSE_BYTES {
            return Err(NetworkError::ResponseTooLarge {
                limit: MAX_DECODED_RESPONSE_BYTES,
            });
        }
        Ok(NetworkResponse {
            request_id: response.request_id,
            status: response.status,
            headers: response.headers,
            body,
        })
    }
}

/// Serve a worker's inherited broker socket from the privileged parent.
pub fn serve_ipc_broker(
    stream: UnixStream,
    mut broker: impl NetworkBroker,
    max_frame_bytes: usize,
) -> Result<(), NetworkError> {
    let reader_stream = stream
        .try_clone()
        .map_err(|error| NetworkError::Transport(error.to_string()))?;
    let mut reader = BufReader::new(reader_stream);
    let mut writer = stream;
    let mut last_message_id = 0_u64;
    loop {
        let mut line = Vec::new();
        let read = read_bounded_line(&mut reader, &mut line, max_frame_bytes)
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        if read == 0 {
            return Ok(());
        }
        let response = if line.len() > max_frame_bytes {
            BrokerResponseEnvelope {
                protocol_version: BROKER_PROTOCOL_VERSION,
                correlation_id: 0,
                response: None,
                error: Some("broker request exceeded the IPC frame limit".into()),
            }
        } else {
            match serde_json::from_slice::<BrokerRequestEnvelope>(&line) {
                Ok(envelope)
                    if envelope.protocol_version == BROKER_PROTOCOL_VERSION
                        && envelope.message_id > last_message_id =>
                {
                    let correlation_id = envelope.message_id;
                    last_message_id = envelope.message_id;
                    let request = envelope.request;
                    let result = BASE64
                        .decode(request.body_base64)
                        .map_err(|error| error.to_string())
                        .and_then(|body| {
                            broker
                                .send(NetworkRequest {
                                    request_id: request.request_id,
                                    method: request.method,
                                    url: request.url,
                                    headers: sanitize_worker_headers(request.headers),
                                    body,
                                    resolved_addrs: Vec::new(),
                                })
                                .map_err(|error| error.to_string())
                        });
                    match result {
                        Ok(response) => BrokerResponseEnvelope {
                            protocol_version: BROKER_PROTOCOL_VERSION,
                            correlation_id,
                            response: Some(EncodedNetworkResponse {
                                request_id: response.request_id,
                                status: response.status,
                                headers: sanitize_parent_response_headers(response.headers),
                                body_base64: BASE64.encode(response.body),
                            }),
                            error: None,
                        },
                        Err(error) => BrokerResponseEnvelope {
                            protocol_version: BROKER_PROTOCOL_VERSION,
                            correlation_id,
                            response: None,
                            error: Some(error.to_string()),
                        },
                    }
                }
                Ok(envelope) => BrokerResponseEnvelope {
                    protocol_version: BROKER_PROTOCOL_VERSION,
                    correlation_id: envelope.message_id,
                    response: None,
                    error: Some(if envelope.protocol_version != BROKER_PROTOCOL_VERSION {
                        "unsupported broker protocol version".into()
                    } else {
                        "duplicate or out-of-order broker message".into()
                    }),
                },
                Err(error) => BrokerResponseEnvelope {
                    protocol_version: BROKER_PROTOCOL_VERSION,
                    correlation_id: 0,
                    response: None,
                    error: Some(format!("invalid broker request: {error}")),
                },
            }
        };
        serde_json::to_writer(&mut writer, &response)
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        writer
            .write_all(b"\n")
            .and_then(|_| writer.flush())
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        if line.len() > max_frame_bytes {
            return Ok(());
        }
    }
}

fn sanitize_worker_headers(headers: Vec<(String, String)>) -> Vec<(String, String)> {
    const ALLOWED: &[&str] = &[
        "accept",
        "accept-language",
        "cache-control",
        "content-type",
        "if-match",
        "if-modified-since",
        "if-none-match",
        "if-unmodified-since",
        "pragma",
        "range",
        "referer",
        "origin",
        "user-agent",
    ];
    headers
        .into_iter()
        .filter(|(name, _)| ALLOWED.contains(&name.to_ascii_lowercase().as_str()))
        .collect()
}

fn sanitize_parent_response_headers(headers: Vec<(String, String)>) -> Vec<(String, String)> {
    const DENIED: &[&str] = &[
        "set-cookie",
        "set-cookie2",
        "proxy-authenticate",
        "proxy-authorization",
    ];
    headers
        .into_iter()
        .filter(|(name, _)| !DENIED.contains(&name.to_ascii_lowercase().as_str()))
        .collect()
}

fn read_bounded_line(
    reader: &mut impl BufRead,
    output: &mut Vec<u8>,
    limit: usize,
) -> std::io::Result<usize> {
    let mut limited = reader.take(limit.saturating_add(1) as u64);
    limited.read_until(b'\n', output)
}

impl<B: NetworkBroker> NetworkBroker for PolicyBroker<B> {
    fn send(&mut self, mut request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        request.resolved_addrs = self.authorize(&request)?;
        self.inner.send(request)
    }
}

#[derive(Default)]
pub struct UnavailableBroker;

impl NetworkBroker for UnavailableBroker {
    fn send(&mut self, _request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        Err(NetworkError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    struct RecordingBroker {
        requests: Arc<Mutex<Vec<NetworkRequest>>>,
        body: Vec<u8>,
    }

    impl NetworkBroker for RecordingBroker {
        fn send(&mut self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            self.requests.lock().unwrap().push(request.clone());
            Ok(NetworkResponse {
                request_id: request.request_id,
                status: 200,
                headers: vec![("content-type".into(), "text/plain".into())],
                body: self.body.clone(),
            })
        }
    }

    fn request(url: &str) -> NetworkRequest {
        NetworkRequest {
            request_id: 1,
            method: "GET".into(),
            url: url.into(),
            headers: Vec::new(),
            body: Vec::new(),
            resolved_addrs: Vec::new(),
        }
    }

    #[test]
    fn broker_enforces_policy_before_transport() {
        let mut broker = PolicyBroker::new(NetworkPolicy::default(), UnavailableBroker);
        assert!(matches!(
            broker.send(request("http://127.0.0.1/private")),
            Err(NetworkError::Policy(PolicyDenied::Loopback))
        ));
        assert!(matches!(
            broker.send(request("file:///etc/passwd")),
            Err(NetworkError::Policy(PolicyDenied::Scheme(_)))
        ));
        assert!(matches!(
            broker.send(request("http://localhost/private")),
            Err(NetworkError::Policy(PolicyDenied::Loopback))
        ));
        assert!(matches!(
            broker.send(request("https://1.1.1.1/")),
            Err(NetworkError::Unavailable)
        ));
    }

    #[test]
    fn named_loopback_policy_allows_local_resolution() {
        let policy = NetworkPolicy {
            allow_loopback: true,
            ..NetworkPolicy::default()
        };
        let mut broker = PolicyBroker::new(policy, UnavailableBroker);
        assert!(matches!(
            broker.send(request("http://localhost:3000/")),
            Err(NetworkError::Unavailable)
        ));
    }

    #[test]
    fn policy_broker_pins_every_vetted_resolution() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let inner = RecordingBroker {
            requests: requests.clone(),
            body: Vec::new(),
        };
        let mut broker = PolicyBroker::new(
            NetworkPolicy {
                allow_loopback: true,
                ..NetworkPolicy::default()
            },
            inner,
        );
        broker
            .send(request("http://localhost:34781/"))
            .expect("loopback test policy permits the vetted address");
        let recorded = requests.lock().unwrap();
        assert!(!recorded[0].resolved_addrs.is_empty());
        assert!(
            recorded[0]
                .resolved_addrs
                .iter()
                .all(|address| address.ip().is_loopback())
        );
    }

    #[test]
    fn worker_broker_is_typed_correlated_and_header_filtered() {
        let (worker, parent) = UnixStream::pair().expect("socket pair");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let server = std::thread::spawn(move || {
            serve_ipc_broker(
                parent,
                RecordingBroker {
                    requests: captured,
                    body: b"brokered".to_vec(),
                },
                MAX_BROKER_FRAME_BYTES,
            )
        });
        let mut broker = IpcNetworkBroker::new(worker);
        let mut network_request = request("https://example.test/");
        network_request.headers = vec![
            ("Accept".into(), "text/plain".into()),
            ("Authorization".into(), "secret".into()),
            ("X-Worker-Secret".into(), "secret".into()),
        ];
        let response = broker.send(network_request).expect("broker response");
        assert_eq!(response.body, b"brokered");
        drop(broker);
        server.join().unwrap().expect("broker server");
        assert_eq!(
            requests.lock().unwrap()[0].headers,
            vec![("Accept".into(), "text/plain".into())]
        );
    }

    #[test]
    fn parent_cookie_headers_are_not_exposed_to_the_worker() {
        assert_eq!(
            sanitize_parent_response_headers(vec![
                ("Content-Type".into(), "text/plain".into()),
                ("Set-Cookie".into(), "session=secret".into()),
                ("Proxy-Authenticate".into(), "secret".into()),
            ]),
            vec![("Content-Type".into(), "text/plain".into())]
        );
    }

    #[test]
    fn parent_broker_rejects_duplicate_message_ids() {
        let (mut worker, parent) = UnixStream::pair().expect("socket pair");
        let server = std::thread::spawn(move || {
            serve_ipc_broker(
                parent,
                RecordingBroker {
                    requests: Arc::new(Mutex::new(Vec::new())),
                    body: b"ok".to_vec(),
                },
                MAX_BROKER_FRAME_BYTES,
            )
        });
        let reader_stream = worker.try_clone().expect("clone worker socket");
        let mut reader = BufReader::new(reader_stream);
        let envelope = BrokerRequestEnvelope {
            protocol_version: BROKER_PROTOCOL_VERSION,
            message_id: 1,
            request: EncodedNetworkRequest {
                request_id: 7,
                method: "GET".into(),
                url: "https://example.test/".into(),
                headers: Vec::new(),
                body_base64: String::new(),
            },
        };
        for attempt in 0..2 {
            serde_json::to_writer(&mut worker, &envelope).expect("write request");
            worker.write_all(b"\n").expect("finish frame");
            worker.flush().expect("flush request");
            let mut line = String::new();
            reader.read_line(&mut line).expect("read response");
            let response: BrokerResponseEnvelope =
                serde_json::from_str(&line).expect("valid response");
            if attempt == 0 {
                assert!(response.error.is_none());
            } else {
                assert_eq!(
                    response.error.as_deref(),
                    Some("duplicate or out-of-order broker message")
                );
            }
        }
        drop(reader);
        drop(worker);
        server.join().unwrap().expect("broker server");
    }

    #[test]
    fn worker_rejects_decoded_response_over_sixteen_mib() {
        let (worker, parent) = UnixStream::pair().expect("socket pair");
        let server = std::thread::spawn(move || {
            serve_ipc_broker(
                parent,
                RecordingBroker {
                    requests: Arc::new(Mutex::new(Vec::new())),
                    body: vec![0; MAX_DECODED_RESPONSE_BYTES + 1],
                },
                MAX_BROKER_FRAME_BYTES,
            )
        });
        let mut broker = IpcNetworkBroker::new(worker);
        assert!(matches!(
            broker.send(request("https://example.test/")),
            Err(NetworkError::ResponseTooLarge {
                limit: MAX_DECODED_RESPONSE_BYTES
            })
        ));
        drop(broker);
        server.join().unwrap().expect("broker server");
    }
}
