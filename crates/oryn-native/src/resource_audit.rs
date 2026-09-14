//! Opt-in, bounded parent-side HTTP evidence. Never retain bodies or headers.
use crate::network::NetworkRequest;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use std::time::Instant;

const MAX_RECORDS: usize = 4096;

#[derive(Clone, Default)]
pub struct ResourceAudit(Arc<Mutex<State>>);

#[derive(Default)]
struct State {
    context_id: Option<u64>,
    started: Option<Instant>,
    overflow: bool,
    records: Vec<ResourceRecord>,
    websockets: Vec<WebSocketRecord>,
    gaps: std::collections::BTreeSet<String>,
}

#[derive(Clone, Serialize)]
pub struct ResourceRecord {
    pub sequence: usize,
    pub page_id: Option<u64>,
    pub worker_pid: Option<u32>,
    pub request_id: u64,
    pub url: String,
    pub query_sha256: Option<String>,
    pub initiator: Option<String>,
    pub destination: String,
    pub method: String,
    pub status: Option<u16>,
    pub mime: Option<String>,
    pub response_sha256: Option<String>,
    pub response_bytes: usize,
    pub started_ms: u128,
    pub elapsed_ms: Option<u128>,
    pub terminal: Option<String>,
}

#[derive(Serialize)]
pub struct ResourceSnapshot {
    pub schema_version: u32,
    pub context_id: Option<u64>,
    pub complete: bool,
    pub overflow: bool,
    pub records: Vec<ResourceRecord>,
    pub websockets: Vec<WebSocketRecord>,
    pub gaps: Vec<String>,
}

#[derive(Clone, Serialize)]
pub struct WebSocketRecord {
    pub page_id: u64,
    pub socket_id: u64,
    pub url: String,
    pub query_sha256: Option<String>,
    pub handshake_status: Option<u16>,
    pub sent_bytes: usize,
    pub received_bytes: usize,
    pub terminal: Option<String>,
}

fn scrub_url(value: &str) -> String {
    let Ok(mut url) = url::Url::parse(value) else {
        return "invalid-url".into();
    };
    if !matches!(url.scheme(), "http" | "https" | "ws" | "wss") {
        return format!("{}:[redacted]", url.scheme());
    }
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    url.to_string()
}

impl ResourceAudit {
    pub fn begin_websocket(
        &self,
        page_id: u64,
        socket_id: u64,
        url: &str,
    ) -> Option<WebSocketObservation> {
        let mut state = self.0.lock().expect("resource audit lock");
        state.started?;
        if state.websockets.len() >= MAX_RECORDS || url.len() > 8192 {
            state.overflow = true;
            return None;
        }
        let sequence = state.websockets.len();
        state.websockets.push(WebSocketRecord {
            page_id,
            socket_id,
            url: scrub_url(url),
            query_sha256: url::Url::parse(url).ok().and_then(|url| {
                url.query()
                    .map(|query| format!("{:x}", Sha256::digest(query.as_bytes())))
            }),
            handshake_status: None,
            sent_bytes: 0,
            received_bytes: 0,
            terminal: None,
        });
        Some(WebSocketObservation {
            audit: self.clone(),
            sequence,
        })
    }
    pub fn enable(&self, context_id: u64) {
        let mut state = self.0.lock().expect("resource audit lock");
        if state.context_id.is_none() {
            state.context_id = Some(context_id);
            state.started = Some(Instant::now());
        }
    }

    pub fn begin(&self, request: &NetworkRequest, page_id: Option<u64>) -> Option<Observation> {
        let mut state = self.0.lock().expect("resource audit lock");
        let start = state.started?;
        if state.records.len() >= MAX_RECORDS
            || request.url.len() > 8192
            || request.method.len() > 128
            || request
                .security
                .initiator_url
                .as_ref()
                .is_some_and(|url| url.len() > 8192)
        {
            state.overflow = true;
            return None;
        }
        let sequence = state.records.len();
        let query_sha256 = url::Url::parse(&request.url).ok().and_then(|url| {
            url.query()
                .map(|query| format!("{:x}", Sha256::digest(query.as_bytes())))
        });
        state.records.push(ResourceRecord {
            sequence,
            page_id,
            worker_pid: None,
            request_id: request.request_id,
            url: scrub_url(&request.url),
            query_sha256,
            initiator: request.security.initiator_url.as_deref().map(scrub_url),
            destination: format!("{:?}", request.security.kind),
            method: request.method.clone(),
            status: None,
            mime: None,
            response_sha256: None,
            response_bytes: 0,
            started_ms: start.elapsed().as_millis(),
            elapsed_ms: None,
            terminal: None,
        });
        Some(Observation {
            audit: self.clone(),
            sequence,
            started: Instant::now(),
            hash: Sha256::new(),
            bytes: 0,
            finished: false,
        })
    }

    pub fn snapshot(&self) -> ResourceSnapshot {
        let state = self.0.lock().expect("resource audit lock");
        ResourceSnapshot {
            schema_version: 2,
            context_id: state.context_id,
            complete: state.context_id.is_some()
                && !state.records.is_empty()
                && !state.overflow
                && state.gaps.is_empty()
                && state
                    .websockets
                    .iter()
                    .all(|record| record.terminal.is_some())
                && state
                    .records
                    .iter()
                    .all(|record| record.terminal.is_some() && record.page_id.is_some()),
            overflow: state.overflow,
            records: state.records.clone(),
            websockets: state.websockets.clone(),
            gaps: state.gaps.iter().cloned().collect(),
        }
    }

    pub fn mark_gap(&self, reason: &str) {
        let mut state = self.0.lock().expect("resource audit lock");
        if state.context_id.is_some() {
            state.gaps.insert(reason.into());
        }
    }

    pub fn cancel(&self, request_id: u64, page_id: Option<u64>) {
        let mut state = self.0.lock().expect("resource audit lock");
        let elapsed = state.started.map(|start| start.elapsed().as_millis());
        for row in &mut state.records {
            if row.request_id == request_id && row.page_id == page_id && row.terminal.is_none() {
                row.terminal = Some("cancelled".into());
                row.elapsed_ms = elapsed.map(|elapsed| elapsed.saturating_sub(row.started_ms));
            }
        }
    }
}

pub struct WebSocketObservation {
    audit: ResourceAudit,
    sequence: usize,
}

impl WebSocketObservation {
    pub fn opened(&mut self, status: u16) {
        self.audit.0.lock().expect("resource audit lock").websockets[self.sequence]
            .handshake_status = Some(status);
    }
    pub fn bytes(&mut self, sent: usize, received: usize) {
        let mut state = self.audit.0.lock().expect("resource audit lock");
        let row = &mut state.websockets[self.sequence];
        row.sent_bytes = row.sent_bytes.saturating_add(sent);
        row.received_bytes = row.received_bytes.saturating_add(received);
    }
    pub fn finish(&mut self, terminal: &str) {
        self.audit.0.lock().expect("resource audit lock").websockets[self.sequence]
            .terminal
            .get_or_insert_with(|| terminal.into());
    }
}

impl Drop for WebSocketObservation {
    fn drop(&mut self) {
        let mut state = self.audit.0.lock().expect("resource audit lock");
        let row = &mut state.websockets[self.sequence];
        let terminal = if row.handshake_status.is_some() {
            "cancelled_or_abandoned"
        } else {
            "failed"
        };
        row.terminal.get_or_insert_with(|| terminal.into());
    }
}

pub struct Observation {
    audit: ResourceAudit,
    sequence: usize,
    started: Instant,
    hash: Sha256,
    bytes: usize,
    finished: bool,
}

impl Observation {
    pub fn bind_worker(&mut self, pid: Option<u32>) {
        self.audit.0.lock().expect("resource audit lock").records[self.sequence].worker_pid = pid;
    }
    pub fn headers(&mut self, status: u16, headers: &[(String, String)]) {
        let mut state = self.audit.0.lock().expect("resource audit lock");
        let row = &mut state.records[self.sequence];
        row.status = Some(status);
        // MIME only: never authorization, cookies, redirect tokens or other headers.
        row.mime = headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            .map(|(_, value)| value.chars().take(256).collect());
    }
    pub fn chunk(&mut self, bytes: &[u8]) {
        self.hash.update(bytes);
        self.bytes += bytes.len();
    }
    pub fn finish(&mut self, terminal: &str) {
        if self.finished {
            return;
        }
        let mut state = self.audit.0.lock().expect("resource audit lock");
        let row = &mut state.records[self.sequence];
        if row.terminal.is_some() {
            self.finished = true;
            return;
        }
        row.terminal = Some(terminal.into());
        row.response_bytes = self.bytes;
        row.elapsed_ms = Some(self.started.elapsed().as_millis());
        if terminal == "complete" {
            row.response_sha256 = Some(format!("{:x}", self.hash.clone().finalize()));
        }
        self.finished = true;
    }
}

impl Drop for Observation {
    fn drop(&mut self) {
        self.finish("cancelled_or_abandoned");
    }
}

pub struct AuditedRead {
    pub inner: Box<dyn std::io::Read + Send>,
    pub observation: Observation,
}
impl std::io::Read for AuditedRead {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self.inner.read(buffer) {
            Ok(0) if !buffer.is_empty() => {
                self.observation.finish("complete");
                Ok(0)
            }
            Ok(size) => {
                self.observation.chunk(&buffer[..size]);
                Ok(size)
            }
            Err(error) => {
                self.observation.finish("failed");
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn websocket_observation_redacts_and_records_terminal_without_payload() {
        let audit = ResourceAudit::default();
        assert!(audit.begin_websocket(1, 1, "ws://example.test/").is_none());
        audit.enable(1);
        {
            let mut observation = audit
                .begin_websocket(7, 3, "wss://user:password@example.test/echo?secret=token")
                .unwrap();
            observation.opened(101);
            observation.bytes(123, 456);
            observation.finish("closed");
        }
        let snapshot = audit.snapshot();
        let record = &snapshot.websockets[0];
        assert_eq!(record.url, "wss://example.test/echo");
        assert_eq!(record.sent_bytes, 123);
        assert_eq!(record.received_bytes, 456);
        assert_eq!(record.terminal.as_deref(), Some("closed"));
        let encoded = serde_json::to_string(&snapshot).unwrap();
        assert!(!encoded.contains("password"));
        assert!(!encoded.contains("secret=token"));
    }

    #[test]
    fn websocket_failed_handshake_and_abandonment_are_not_success() {
        let audit = ResourceAudit::default();
        audit.enable(1);
        drop(audit.begin_websocket(1, 1, "ws://example.test/"));
        let mut live = audit.begin_websocket(1, 2, "ws://example.test/").unwrap();
        live.opened(101);
        drop(live);
        let snapshot = audit.snapshot();
        assert_eq!(snapshot.websockets[0].terminal.as_deref(), Some("failed"));
        assert_eq!(
            snapshot.websockets[1].terminal.as_deref(),
            Some("cancelled_or_abandoned")
        );
    }
    use crate::network::{
        NetworkBroker, NetworkError, NetworkResponse, SharedNetworkBroker,
        collect_operation_bounded,
    };
    use std::time::Duration;

    fn request() -> NetworkRequest {
        NetworkRequest {
            request_id: 1,
            method: "GET".into(),
            url: "https://user:password@example.test/helper.js?secret=token#private".into(),
            headers: vec![("authorization".into(), "private-header".into())],
            body: Vec::new(),
            credentials: true,
            security: Default::default(),
            resolved_addrs: Vec::new(),
        }
    }
    struct Fixture;
    impl NetworkBroker for Fixture {
        fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
            Ok(NetworkResponse {
                request_id: request.request_id,
                status: 200,
                headers: vec![
                    ("content-type".into(), "text/javascript".into()),
                    ("set-cookie".into(), "private-cookie".into()),
                ],
                body: b"private-body".to_vec(),
            })
        }
    }

    #[test]
    fn audit_is_opt_in_and_redacts_payloads_and_credentials() {
        let broker = SharedNetworkBroker::new(Fixture).for_page(7);
        broker.send(request()).unwrap();
        assert!(broker.resource_audit().snapshot().records.is_empty());
        broker.resource_audit().enable(2);
        broker.send(request()).unwrap();
        let snapshot = broker.resource_audit().snapshot();
        assert!(snapshot.complete);
        let row = &snapshot.records[0];
        assert_eq!(row.page_id, Some(7));
        assert_eq!(row.url, "https://example.test/helper.js");
        assert_eq!(
            row.response_sha256.as_deref(),
            Some(format!("{:x}", Sha256::digest(b"private-body")).as_str())
        );
        let serialized = serde_json::to_string(&snapshot).unwrap();
        for secret in [
            "password",
            "private-header",
            "private-cookie",
            "private-body",
            "secret=token",
            "#private",
        ] {
            assert!(!serialized.contains(secret), "{secret}");
        }
    }

    #[test]
    fn streaming_hash_preserves_transport_and_cancellation_is_terminal() {
        let broker = SharedNetworkBroker::new(Fixture).for_page(9);
        broker.resource_audit().enable(3);
        let response =
            collect_operation_bounded(&broker, request(), Duration::from_secs(2), 1024).unwrap();
        assert_eq!(response.body, b"private-body");
        assert!(broker.resource_audit().snapshot().complete);
        let mut observation = broker.resource_audit().begin(&request(), Some(9)).unwrap();
        broker.cancel_operation(1).unwrap();
        observation.finish("complete");
        assert_eq!(
            broker.resource_audit().snapshot().records[1]
                .terminal
                .as_deref(),
            Some("cancelled")
        );
    }

    #[test]
    fn missing_binding_pending_and_overflow_are_not_complete() {
        let audit = ResourceAudit::default();
        audit.enable(1);
        assert!(!audit.snapshot().complete);
        let pending = audit.begin(&request(), None).unwrap();
        assert!(!audit.snapshot().complete);
        drop(pending);
        assert!(!audit.snapshot().complete);
        for _ in 1..MAX_RECORDS {
            drop(audit.begin(&request(), Some(1)));
        }
        assert!(audit.begin(&request(), Some(1)).is_none());
        assert!(audit.snapshot().overflow);
        assert!(!audit.snapshot().complete);
    }
}
