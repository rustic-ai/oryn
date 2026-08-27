use std::{
    io::Read,
    net::{IpAddr, ToSocketAddrs},
    sync::{Arc, Mutex},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use url::Url;

use crate::security::{NetworkPolicy, PolicyDenied};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkRequest {
    pub request_id: u64,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkResponse {
    pub request_id: u64,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

pub trait NetworkBroker {
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
    inner: Option<Arc<Mutex<PolicyBroker<RustlsTransport>>>>,
}

impl SharedNetworkBroker {
    pub fn new(broker: PolicyBroker<RustlsTransport>) -> Self {
        Self {
            inner: Some(Arc::new(Mutex::new(broker))),
        }
    }

    pub fn lock(
        &self,
    ) -> std::sync::LockResult<std::sync::MutexGuard<'_, PolicyBroker<RustlsTransport>>> {
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

    fn authorize(&self, request: &NetworkRequest) -> Result<(), NetworkError> {
        let url = Url::parse(&request.url)
            .map_err(|error| NetworkError::InvalidUrl(error.to_string()))?;
        self.policy.check_scheme(url.scheme())?;
        if let Some(host) = url.host_str() {
            if (host.eq_ignore_ascii_case("localhost") || host.ends_with(".localhost"))
                && !self.policy.allow_loopback
            {
                return Err(NetworkError::Policy(PolicyDenied::Loopback));
            }
            if let Ok(address) = host.parse::<IpAddr>() {
                self.policy.check_ip(address)?;
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
                }
                if !resolved {
                    return Err(NetworkError::Resolution {
                        host: host.to_string(),
                        detail: "resolver returned no addresses".into(),
                    });
                }
            }
        }
        Ok(())
    }
}

/// A single-request rustls transport. Redirects are intentionally disabled so
/// callers can send every hop back through `PolicyBroker`.
pub struct RustlsTransport {
    client: reqwest::blocking::Client,
    max_response_bytes: usize,
}

impl RustlsTransport {
    pub fn new(timeout: Duration, max_response_bytes: usize) -> Result<Self, NetworkError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .cookie_store(true)
            .build()
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        Ok(Self {
            client,
            max_response_bytes,
        })
    }
}

impl NetworkBroker for RustlsTransport {
    fn send(&mut self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|error| NetworkError::Transport(error.to_string()))?;
        let mut builder = self.client.request(method, &request.url);
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

impl<B: NetworkBroker> NetworkBroker for PolicyBroker<B> {
    fn send(&mut self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        self.authorize(&request)?;
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
    use super::*;

    fn request(url: &str) -> NetworkRequest {
        NetworkRequest {
            request_id: 1,
            method: "GET".into(),
            url: url.into(),
            headers: Vec::new(),
            body: Vec::new(),
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
}
