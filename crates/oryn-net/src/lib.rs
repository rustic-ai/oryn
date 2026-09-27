//! Transport-neutral asynchronous browser networking contracts.

use std::{collections::BTreeMap, net::SocketAddr, pin::Pin, sync::Arc, time::Duration};

use async_trait::async_trait;
use bytes::Bytes;
use futures_core::Stream;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, BufReader};
use tokio::sync::Mutex;
use tokio_util::io::{ReaderStream, StreamReader};
use tokio_util::sync::CancellationToken;

pub mod cache;
pub mod cors;
pub mod fetch;
pub mod mime_sniff;

pub type BodyStream = Pin<Box<dyn Stream<Item = Result<Bytes, TransportError>> + Send>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DnsTiming {
    pub start: std::time::Instant,
    pub end: std::time::Instant,
}
/// Monotonic offsets from this hop's native admission. Missing phases mean no
/// new connection/handshake was established for this request.
pub use oryn_common::transport_timing::TransportTiming;
fn phase_offset(start: std::time::Instant, phase: Option<std::time::Instant>) -> Option<u64> {
    phase
        .and_then(|phase| phase.checked_duration_since(start))
        .map(|time| time.as_micros().min(u64::MAX as u128) as u64)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpRequest {
    #[serde(skip)]
    pub dns_timing: Option<DnsTiming>,
    pub operation_id: u64,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    #[serde(default)]
    pub resolved_addrs: Vec<SocketAddr>,
}

pub struct HttpResponse {
    pub timing: TransportTiming,
    pub upload: Option<UploadProgress>,
    pub operation_id: u64,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: BodyStream,
}

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("transport unavailable")]
    Unavailable,
    #[error("transport cancelled")]
    Cancelled,
    #[error("server certificate rejected: {0}")]
    CertificateRejected(String),
    #[error("transport failed: {0}")]
    Failed(String),
    #[error("compressed response exceeded {limit} bytes")]
    CompressedLimit { limit: usize },
    #[error("decoded response exceeded {limit} bytes")]
    DecodedLimit { limit: usize },
    #[error("unsupported content encoding {0:?}")]
    UnsupportedContentEncoding(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentEncoding {
    Identity,
    Gzip,
    Deflate,
    Brotli,
    Zstd,
}

impl ContentEncoding {
    pub fn parse(value: Option<&str>) -> Result<Self, TransportError> {
        match value
            .unwrap_or("identity")
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "" | "identity" => Ok(Self::Identity),
            "gzip" | "x-gzip" => Ok(Self::Gzip),
            "deflate" => Ok(Self::Deflate),
            "br" => Ok(Self::Brotli),
            "zstd" => Ok(Self::Zstd),
            value => Err(TransportError::UnsupportedContentEncoding(value.into())),
        }
    }
}

#[async_trait]
pub trait HttpTransport: Send + Sync {
    async fn start(&self, request: HttpRequest) -> Result<HttpResponse, TransportError>;
    async fn cancel(&self, operation_id: u64) -> Result<(), TransportError>;
    fn upload_progress(&self, _operation_id: u64) -> Option<UploadProgress> {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct UploadProgress {
    pub bytes_consumed: u64,
    pub total_bytes: u64,
    pub body_complete: bool,
}

struct PendingTransportOperation {
    cancellation: CancellationToken,
    upload_consumed: std::sync::atomic::AtomicU64,
    upload_total: u64,
}

type PendingTransportOperations =
    Arc<std::sync::Mutex<BTreeMap<u64, Arc<PendingTransportOperation>>>>;

/// Owns registration from request submission through response-body disposal.
/// Dropping either the start future or the unpolled body releases registration.
struct TransportOperationLease {
    pending: PendingTransportOperations,
    id: u64,
    operation: Arc<PendingTransportOperation>,
}

impl TransportOperationLease {
    fn register(
        pending: &PendingTransportOperations,
        id: u64,
        upload_total: u64,
    ) -> Result<Self, TransportError> {
        let operation = Arc::new(PendingTransportOperation {
            cancellation: CancellationToken::new(),
            upload_consumed: std::sync::atomic::AtomicU64::new(0),
            upload_total,
        });
        let mut registrations = pending.lock().unwrap_or_else(|error| error.into_inner());
        if registrations.contains_key(&id) {
            return Err(TransportError::Failed(
                "duplicate active transport operation".into(),
            ));
        }
        registrations.insert(id, operation.clone());
        Ok(Self {
            pending: pending.clone(),
            id,
            operation,
        })
    }
}

impl PendingTransportOperation {
    fn upload_progress(&self) -> UploadProgress {
        let bytes_consumed = self
            .upload_consumed
            .load(std::sync::atomic::Ordering::Acquire);
        UploadProgress {
            bytes_consumed,
            total_bytes: self.upload_total,
            body_complete: bytes_consumed == self.upload_total,
        }
    }
}

fn tracked_upload_body(mut bytes: Bytes, operation: Arc<PendingTransportOperation>) -> BodyStream {
    Box::pin(async_stream::try_stream! {
        while !bytes.is_empty() {
            if operation.cancellation.is_cancelled() { Err(TransportError::Cancelled)?; }
            let chunk = bytes.split_to(bytes.len().min(64 * 1024));
            operation.upload_consumed.fetch_add(chunk.len() as u64, std::sync::atomic::Ordering::Release);
            yield chunk;
        }
    })
}

impl Drop for TransportOperationLease {
    fn drop(&mut self) {
        self.operation.cancellation.cancel();
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if pending
            .get(&self.id)
            .is_some_and(|entry| Arc::ptr_eq(entry, &self.operation))
        {
            pending.remove(&self.id);
        }
    }
}

/// Reusable asynchronous rustls transport. Redirects and content decoding are
/// disabled here because the browser Fetch layer must inspect every hop and
/// enforce both compressed and decoded byte limits itself.
pub struct RustlsTransport {
    clients: Arc<Mutex<BTreeMap<String, reqwest::Client>>>,
    pending: PendingTransportOperations,
    max_body_bytes: usize,
    timeout: Duration,
    force_http2_prior_knowledge: bool,
    explicit_roots: Option<Vec<reqwest::Certificate>>,
}

impl RustlsTransport {
    pub fn new(timeout: Duration, max_body_bytes: usize) -> Result<Self, TransportError> {
        Ok(Self {
            clients: Arc::new(Mutex::new(BTreeMap::new())),
            pending: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
            max_body_bytes,
            timeout,
            force_http2_prior_knowledge: false,
            explicit_roots: None,
        })
    }

    /// Immutable, caller-authorized trust for an explicit fixture context. This
    /// mode never reads legacy WPT environment variables or disables certificate
    /// name/chain validation. An empty list retains ordinary platform roots.
    pub fn with_trusted_roots(mut self, roots: &[Vec<u8>]) -> Result<Self, TransportError> {
        if roots.len() > 16 || roots.iter().any(|pem| pem.len() > 1024 * 1024) {
            return Err(TransportError::Failed(
                "explicit CA configuration exceeds limits".into(),
            ));
        }
        let mut certificates = Vec::new();
        for pem in roots {
            let parsed = reqwest::Certificate::from_pem_bundle(pem)
                .map_err(|_| TransportError::Failed("invalid explicit CA certificate".into()))?;
            if parsed.is_empty() {
                return Err(TransportError::Failed(
                    "explicit CA input contains no certificates".into(),
                ));
            }
            certificates.extend(parsed);
        }
        if !certificates.is_empty() {
            // from_pem only stores bytes with reqwest's rustls backend. Building
            // validates DER/root-store admission without making any request.
            let mut validation = reqwest::Client::builder().use_rustls_tls().no_proxy();
            for certificate in &certificates {
                validation = validation.add_root_certificate(certificate.clone());
            }
            validation
                .build()
                .map_err(|_| TransportError::Failed("invalid explicit CA certificate".into()))?;
        }
        // Consuming configuration invalidates previously built clients; no client
        // carrying an earlier trust configuration may survive this transition.
        self.clients = Arc::new(Mutex::new(BTreeMap::new()));
        self.explicit_roots = Some(certificates);
        Ok(self)
    }

    pub fn with_http2_prior_knowledge(mut self) -> Self {
        self.force_http2_prior_knowledge = true;
        self
    }

    async fn client_for(&self, request: &HttpRequest) -> Result<reqwest::Client, TransportError> {
        let url = url::Url::parse(&request.url)
            .map_err(|error| TransportError::Failed(error.to_string()))?;
        let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
        let key = format!(
            "{}|{}|{}",
            host,
            request
                .resolved_addrs
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(","),
            self.force_http2_prior_knowledge
        );
        if let Some(client) = self.clients.lock().await.get(&key) {
            return Ok(client.clone());
        }
        // Workspace feature unification may also enable reqwest default TLS.
        // Keep this transport on the backend that owns our measured phases.
        let mut builder = reqwest::Client::builder()
            .use_rustls_tls()
            .timeout(self.timeout)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .no_zstd();
        if let Some(roots) = &self.explicit_roots {
            for certificate in roots {
                builder = builder.add_root_certificate(certificate.clone());
            }
        } else {
            if let Some(certificate_path) = std::env::var_os("ORYN_WPT_CA_CERT") {
                let certificate_bytes = std::fs::read(&certificate_path).map_err(|error| {
                    TransportError::Failed(format!("read WPT CA certificate: {error}"))
                })?;
                let certificate =
                    reqwest::Certificate::from_pem(&certificate_bytes).map_err(|error| {
                        TransportError::Failed(format!("parse WPT CA certificate: {error}"))
                    })?;
                builder = builder.add_root_certificate(certificate);
            }
            if std::env::var_os("ORYN_WPT_CA_CERT").is_some()
                && std::env::var("ORYN_WPT_ALLOW_INVALID_CERTS").as_deref() == Ok("1")
            {
                // This is used only by the pinned loopback WPT harness, whose
                // immutable certificate names web-platform.test while the local
                // policy deliberately exposes localhost/127.0.0.1 origins.
                builder = builder.danger_accept_invalid_certs(true);
            }
            if std::env::var_os("ORYN_WPT_CA_CERT").is_some() && !self.force_http2_prior_knowledge {
                // A small legacy WPT slice asserts the HTTP/1.1 Connection header
                // itself. Its controlled server is intentionally HTTP/1-shaped;
                // keep that harness transport on HTTP/1 without weakening normal
                // production ALPN or the dedicated HTTP/2 conformance path.
                builder = builder.http1_only();
            }
        }
        if self.force_http2_prior_knowledge {
            builder = builder.http2_prior_knowledge();
        }
        if !request.resolved_addrs.is_empty() && host.parse::<std::net::IpAddr>().is_err() {
            builder = builder.resolve_to_addrs(&host, &request.resolved_addrs);
        }
        let client = builder
            .build()
            .map_err(|error| TransportError::Failed(error.to_string()))?;
        self.clients.lock().await.insert(key, client.clone());
        Ok(client)
    }
}

// Header strings carry WebIDL ByteString code points, not UTF-8 payloads.
// Keep this conversion at the transport boundary; URLs and bodies have their
// own encodings and must never pass through it.
fn encode_header_fields(
    fields: &[(String, String)],
) -> Result<reqwest::header::HeaderMap, TransportError> {
    let mut headers = reqwest::header::HeaderMap::new();
    for (name, value) in fields {
        let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
            .map_err(|error| TransportError::Failed(error.to_string()))?;
        let bytes = value
            .chars()
            .map(|character| u8::try_from(u32::from(character)))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| TransportError::Failed("header value is not a ByteString".into()))?;
        let value = reqwest::header::HeaderValue::from_bytes(&bytes)
            .map_err(|error| TransportError::Failed(error.to_string()))?;
        headers.append(name, value);
    }
    // Fetch's HTTP-network step adds this after service-worker interception.
    if headers.contains_key(reqwest::header::RANGE)
        && !headers.contains_key(reqwest::header::ACCEPT_ENCODING)
    {
        headers.insert(
            reqwest::header::ACCEPT_ENCODING,
            reqwest::header::HeaderValue::from_static("identity"),
        );
    }
    Ok(headers)
}
fn decode_header_fields(headers: &reqwest::header::HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                value.as_bytes().iter().copied().map(char::from).collect(),
            )
        })
        .collect()
}

#[cfg(test)]
mod header_byte_tests {
    use super::*;
    #[test]
    fn header_bytes_roundtrip_obs_text_and_preserve_duplicate_fields() {
        let fields = vec![
            ("x-native".into(), "\u{80}\u{e9}\u{ff}".into()),
            ("x-native".into(), "second".into()),
            ("set-cookie".into(), "a=1".into()),
            ("set-cookie".into(), "b=2".into()),
        ];
        let encoded = encode_header_fields(&fields).unwrap();
        assert_eq!(
            encoded
                .get_all("x-native")
                .iter()
                .map(|value| value.as_bytes())
                .collect::<Vec<_>>(),
            vec![&[0x80, 0xe9, 0xff][..], b"second".as_slice()]
        );
        let decoded = decode_header_fields(&encoded);
        for name in ["x-native", "set-cookie"] {
            assert_eq!(
                decoded
                    .iter()
                    .filter(|(key, _)| key == name)
                    .collect::<Vec<_>>(),
                fields
                    .iter()
                    .filter(|(key, _)| key == name)
                    .collect::<Vec<_>>()
            );
        }
    }
    #[test]
    fn range_identity_encoding_is_inserted_only_at_transport_boundary() {
        let source = vec![("Range".into(), "bytes=0-".into())];
        let encoded = encode_header_fields(&source).unwrap();
        assert_eq!(encoded["accept-encoding"], "identity");
        assert_eq!(source.len(), 1);
        let explicit = vec![
            ("range".into(), "bytes=4-".into()),
            ("accept-encoding".into(), "gzip".into()),
        ];
        assert_eq!(
            encode_header_fields(&explicit).unwrap()["accept-encoding"],
            "gzip"
        );
        assert!(
            !encode_header_fields(&[])
                .unwrap()
                .contains_key("accept-encoding")
        );
    }
    #[test]
    fn headers_reject_non_byte_scalars_and_invalid_protocol_bytes() {
        for value in ["\u{100}", "\u{1f600}", "nul\0", "bad\r\nheader"] {
            assert!(encode_header_fields(&[("x-native".into(), value.into())]).is_err());
        }
        assert!(encode_header_fields(&[("bad name".into(), "value".into())]).is_err());
    }
}

#[async_trait]
impl HttpTransport for RustlsTransport {
    async fn start(&self, mut request: HttpRequest) -> Result<HttpResponse, TransportError> {
        let operation = TransportOperationLease::register(
            &self.pending,
            request.operation_id,
            request.body.len() as u64,
        )?;
        let cancellation = operation.operation.cancellation.clone();
        let admitted = std::time::Instant::now();
        if request.resolved_addrs.is_empty() {
            let url = url::Url::parse(&request.url)
                .map_err(|error| TransportError::Failed(error.to_string()))?;
            if let Some(host) = url
                .host_str()
                .filter(|host| host.parse::<std::net::IpAddr>().is_err())
            {
                let start = std::time::Instant::now();
                let port = url
                    .port_or_known_default()
                    .ok_or_else(|| TransportError::Failed("missing request port".into()))?;
                request.resolved_addrs = tokio::select! {_ = cancellation.cancelled()=>return Err(TransportError::Cancelled),addresses=tokio::net::lookup_host((host,port))=>addresses.map_err(|error|TransportError::Failed(error.to_string()))?.collect()};
                request.dns_timing = Some(DnsTiming {
                    start,
                    end: std::time::Instant::now(),
                });
            }
        }
        let timing_origin = request.dns_timing.map_or(admitted, |dns| dns.start);
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|error| TransportError::Failed(error.to_string()))?;
        let client = self.client_for(&request).await?;
        let mut builder = client.request(method, &request.url);
        if !self.force_http2_prior_knowledge
            && request
                .headers
                .iter()
                .all(|(name, _)| !name.eq_ignore_ascii_case("connection"))
        {
            // Browser HTTP/1.1 stacks keep connections persistent by default
            // and expose that transport header to the origin. Supplying it
            // explicitly keeps the async reqwest transport browser-shaped;
            // HTTP/2 deliberately omits this hop-by-hop field.
            builder = builder.header("connection", "keep-alive");
        }
        builder = builder.headers(encode_header_fields(&request.headers)?);

        if !request.body.is_empty() {
            if !request
                .headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            {
                builder = builder.header("content-length", request.body.len());
            }
            builder = builder.body(reqwest::Body::wrap_stream(tracked_upload_body(
                Bytes::from(request.body),
                operation.operation.clone(),
            )));
        }
        let cancellation = operation.operation.cancellation.clone();
        let response = tokio::select! {
            _ = cancellation.cancelled() => return Err(TransportError::Cancelled),
            response = builder.send() => response.map_err(request_error)?,
        };
        let connection = response.extensions().get::<reqwest::ConnectionTiming>();
        let new_connection = connection
            .and_then(|timing| phase_offset(timing_origin, timing.connect_end))
            .is_some();
        let dns = if new_connection {
            request.dns_timing
        } else {
            None
        };
        let timing = TransportTiming {
            encoded_body_size: None,
            measured_response: true,
            worker_start_clock_micros: None,
            delivery: oryn_common::transport_timing::ResponseDelivery::Network,
            exposure: None,
            response_end: None,
            dns_start: dns.and_then(|dns| phase_offset(timing_origin, Some(dns.start))),
            dns_end: dns.and_then(|dns| phase_offset(timing_origin, Some(dns.end))),
            connect_start: phase_offset(
                timing_origin,
                connection.and_then(|timing| timing.connect_start),
            ),
            connect_end: phase_offset(
                timing_origin,
                connection.and_then(|timing| timing.connect_end),
            ),
            secure_start: phase_offset(
                timing_origin,
                connection.and_then(|timing| timing.secure_start),
            ),
            secure_end: phase_offset(
                timing_origin,
                connection.and_then(|timing| timing.secure_end),
            ),
            request_start: phase_offset(
                timing_origin,
                connection.and_then(|timing| timing.secure_end.or(timing.connect_end)),
            )
            .or_else(|| phase_offset(timing_origin, Some(admitted)))
            .unwrap_or(0),
            response_start: phase_offset(timing_origin, Some(std::time::Instant::now()))
                .unwrap_or(0),
            next_hop_protocol: match response.version() {
                reqwest::Version::HTTP_2 => "h2",
                reqwest::Version::HTTP_11 => "http/1.1",
                reqwest::Version::HTTP_10 => "http/1.0",
                _ => "",
            }
            .into(),
        };
        let status = response.status().as_u16();
        let headers = decode_header_fields(response.headers());
        let operation_id = request.operation_id;
        let limit = self.max_body_bytes;
        let mut source = response.bytes_stream();
        let upload = Some(operation.operation.upload_progress());
        let body = Box::pin(async_stream::try_stream! {
            let _operation = operation;
            let mut received = 0_usize;
            loop {
                let item = tokio::select! {
                    _ = cancellation.cancelled() => Err(TransportError::Cancelled),
                    item = source.next() => Ok(item),
                }?;
                let Some(item) = item else { break; };
                let chunk = item.map_err(|error| TransportError::Failed(error.to_string()))?;
                received = received.saturating_add(chunk.len());
                if received > limit {
                    Err(TransportError::Failed(format!("response exceeded {limit} bytes")))?;
                }
                yield chunk;
            }
        });
        Ok(HttpResponse {
            timing,
            upload,
            operation_id,
            status,
            headers,
            body,
        })
    }

    fn upload_progress(&self, operation_id: u64) -> Option<UploadProgress> {
        self.pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&operation_id)
            .map(|operation| operation.upload_progress())
    }

    async fn cancel(&self, operation_id: u64) -> Result<(), TransportError> {
        let operation = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&operation_id);
        if let Some(operation) = operation {
            operation.cancellation.cancel();
            Ok(())
        } else {
            Err(TransportError::Cancelled)
        }
    }
}

fn response_header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn response_has_nosniff(headers: &[(String, String)]) -> bool {
    response_header(headers, "x-content-type-options").is_some_and(|value| {
        value
            .split(',')
            .next()
            .unwrap_or_default()
            .trim_matches([' ', '\t', '\r', '\n'])
            .eq_ignore_ascii_case("nosniff")
    })
}

/// Fetch's script MIME blocking and X-Content-Type-Options checks.
pub fn script_response_mime_allowed(headers: &[(String, String)], module: bool) -> bool {
    let mime = oryn_common::mime_type::MimeType::from_headers(headers);
    let javascript = mime.as_ref().is_some_and(|mime| mime.is_javascript());
    if module || response_has_nosniff(headers) {
        return javascript;
    }
    !mime.is_some_and(|mime| {
        let essence = mime.essence();
        essence.starts_with("audio/")
            || essence.starts_with("image/")
            || essence.starts_with("video/")
            || essence == "text/csv"
    })
}

/// Fetch's nosniff gate for a stylesheet destination. CSS loading can impose
/// additional MIME restrictions independently of this response-header gate.
pub fn stylesheet_response_mime_allowed(headers: &[(String, String)]) -> bool {
    !response_has_nosniff(headers)
        || oryn_common::mime_type::MimeType::from_headers(headers)
            .is_some_and(|mime| mime.essence() == "text/css")
}

/// HTML's linked stylesheet MIME check, after Fetch's independent nosniff gate.
/// Only a quirks document with a CORS-same-origin response may treat another
/// MIME as CSS. Empty sheets follow the same admission rule as nonempty sheets.
pub fn stylesheet_response_allowed(
    headers: &[(String, String)],
    quirks: bool,
    cors_same_origin: bool,
) -> bool {
    stylesheet_response_mime_allowed(headers)
        && (quirks && cors_same_origin
            || oryn_common::mime_type::MimeType::from_headers(headers)
                .is_some_and(|mime| mime.essence() == "text/css"))
}

pub fn decode_text(bytes: &[u8], content_type: Option<&str>) -> (String, &'static str) {
    decode_resource_text(bytes, content_type, TextResourceKind::Fetch)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextResourceKind {
    Html,
    PlainText,
    Xml,
    Css,
    JavaScript,
    ModuleJavaScript,
    Fetch,
}

/// Whether an HTML byte decoder may still accept a parser-discovered label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HtmlEncodingConfidence {
    Tentative,
    Certain,
}

/// Incremental browser text decoder. It buffers only the HTML charset
/// prescan window and then retains `encoding_rs` state across network chunks.
const HTML_DETECTION_SAMPLE_BYTES: usize = 1024;
struct HtmlDetection {
    detector: Box<chardetng::EncodingDetector>,
    sample: Vec<u8>,
}

pub struct IncrementalTextDecoder {
    content_type: Option<String>,
    kind: TextResourceKind,
    prescan: Vec<u8>,
    decoder: Option<encoding_rs::Decoder>,
    encoding_name: Option<&'static str>,
    fallback: Option<&'static encoding_rs::Encoding>,
    html_confidence: HtmlEncodingConfidence,
    detect_html: bool,
    detection: Option<HtmlDetection>,
}

impl IncrementalTextDecoder {
    pub fn new(content_type: Option<&str>, kind: TextResourceKind) -> Self {
        Self {
            content_type: content_type.map(str::to_owned),
            kind,
            prescan: Vec::new(),
            decoder: None,
            encoding_name: None,
            fallback: None,
            detect_html: false,
            detection: None,
            html_confidence: if kind == TextResourceKind::Html {
                HtmlEncodingConfidence::Tentative
            } else {
                HtmlEncodingConfidence::Certain
            },
        }
    }
    pub fn with_fallback(
        content_type: Option<&str>,
        kind: TextResourceKind,
        fallback: Option<&str>,
    ) -> Self {
        let mut decoder = Self::new(content_type, kind);
        decoder.fallback = resource_fallback(kind, fallback);
        decoder
    }

    /// Statistical detection is an explicit native navigation policy. Fetch,
    /// worker/module decoders and inherited document encodings do not opt in.
    pub fn with_html_detection(mut self) -> Self {
        self.detect_html = matches!(
            self.kind,
            TextResourceKind::Html | TextResourceKind::PlainText
        );
        self
    }

    pub fn push(&mut self, bytes: &[u8]) -> String {
        if self.decoder.is_none() {
            let prefix = bytes.len().min(1024 - self.prescan.len());
            self.prescan.extend_from_slice(&bytes[..prefix]);
            let declaration_ready = matches!(self.kind, TextResourceKind::Xml)
                && self.prescan.len() >= 5
                && (!self.prescan.starts_with(b"<?xml")
                    || self.prescan.windows(2).any(|window| window == b"?>"));
            let header_ready = self.prescan.len() >= 4
                && self
                    .content_type
                    .as_deref()
                    .and_then(oryn_common::mime_type::MimeType::parse)
                    .and_then(|mime| {
                        mime.parameter("charset")
                            .and_then(|label| encoding_rs::Encoding::for_label(label.as_bytes()))
                    })
                    .is_some();
            if self.prescan.len() < 1024 && !declaration_ready && !header_ready {
                return String::new();
            }
            let mut output = self.start(false);
            output.push_str(&self.decode(&bytes[prefix..], false));
            return output;
        }
        self.decode(bytes, false)
    }

    pub fn finish(&mut self) -> String {
        if self.decoder.is_none() {
            return self.start(true);
        }
        self.decode(&[], true)
    }

    pub fn html_encoding_confidence(&self) -> HtmlEncodingConfidence {
        self.html_confidence
    }
    /// The input owner calls this before irreversible parser/author effects.
    pub fn commit_html_encoding(&mut self) {
        self.html_confidence = HtmlEncodingConfidence::Certain;
    }
    /// Read-only preflight lets the parser restore its policy before mutating
    /// either decoder state or the retained original bytes.
    pub fn html_encoding_restart_needed(&self, label: &str) -> bool {
        if self.kind != TextResourceKind::Html
            || self.html_confidence == HtmlEncodingConfidence::Certain
        {
            return false;
        }
        let Some(mut encoding) = encoding_rs::Encoding::for_label(label.as_bytes()) else {
            return false;
        };
        if encoding == encoding_rs::UTF_16LE || encoding == encoding_rs::UTF_16BE {
            encoding = encoding_rs::UTF_8;
        }
        if encoding == encoding_rs::X_USER_DEFINED {
            encoding = encoding_rs::WINDOWS_1252;
        }
        self.encoding_name != Some(encoding.name())
            || self
                .detection
                .as_ref()
                .is_some_and(|detection| !detection.sample.is_empty())
    }
    /// Replay uses the exact admitted raw prefix, owned and bounded by the input
    /// adapter. It neither reads the transport nor enlarges the prescan window.
    pub fn restart_html_encoding(
        &mut self,
        label: &str,
        raw_prefix: &[u8],
        last: bool,
    ) -> Option<String> {
        if self.kind != TextResourceKind::Html
            || self.html_confidence == HtmlEncodingConfidence::Certain
        {
            return None;
        }
        let mut encoding = encoding_rs::Encoding::for_label(label.as_bytes())?;
        if encoding == encoding_rs::UTF_16LE || encoding == encoding_rs::UTF_16BE {
            encoding = encoding_rs::UTF_8;
        }
        if encoding == encoding_rs::X_USER_DEFINED {
            encoding = encoding_rs::WINDOWS_1252;
        }
        let buffered_detection = self
            .detection
            .as_ref()
            .is_some_and(|detection| !detection.sample.is_empty());
        self.html_confidence = HtmlEncodingConfidence::Certain;
        self.detection = None;
        self.detect_html = false;
        if self.encoding_name == Some(encoding.name()) && !buffered_detection {
            return None;
        }
        self.prescan.clear();
        self.encoding_name = Some(encoding.name());
        self.decoder = Some(encoding.new_decoder_without_bom_handling());
        Some(self.decode(raw_prefix, last))
    }

    pub fn encoding_name(&self) -> Option<&'static str> {
        self.encoding_name
    }

    fn start(&mut self, last: bool) -> String {
        let bytes = std::mem::take(&mut self.prescan);
        let encoding = select_encoding(
            &bytes,
            self.content_type.as_deref(),
            self.kind,
            self.fallback,
        );
        let bom_length = resource_bom_length(&bytes, self.kind);
        let authoritative = encoding_rs::Encoding::for_bom(&bytes).is_some()
            || self
                .content_type
                .as_deref()
                .and_then(oryn_common::mime_type::MimeType::parse)
                .and_then(|mime| {
                    mime.parameter("charset")
                        .and_then(|label| encoding_rs::Encoding::for_label(label.as_bytes()))
                })
                .is_some();
        if self.kind == TextResourceKind::Html && authoritative {
            self.html_confidence = HtmlEncodingConfidence::Certain;
        }
        if self.detect_html
            && !authoritative
            && self.fallback.is_none()
            && (self.kind == TextResourceKind::PlainText
                || (self.kind == TextResourceKind::Html
                    && self.html_confidence == HtmlEncodingConfidence::Tentative
                    && html_meta_charset(&bytes).is_none()))
        {
            self.detection = Some(HtmlDetection {
                detector: Box::new(chardetng::EncodingDetector::new()),
                sample: Vec::new(),
            });
        }
        self.encoding_name = Some(encoding.name());
        // Selection consumed the one leading BOM. A second BOM is content.
        self.decoder = Some(encoding.new_decoder_without_bom_handling());
        self.decode(&bytes[bom_length..], last)
    }

    fn decode(&mut self, bytes: &[u8], last: bool) -> String {
        let Some(mut detection) = self.detection.take() else {
            return self.decode_selected(bytes, last);
        };
        let mut output = String::new();
        let mut remaining = bytes;
        if detection.sample.is_empty() {
            let ascii = bytes
                .iter()
                .position(|byte| !byte.is_ascii() || *byte == 0x1b)
                .unwrap_or(bytes.len());
            detection.detector.feed(&bytes[..ascii], false);
            output = self.decode_selected(&bytes[..ascii], false);
            remaining = &bytes[ascii..];
            if remaining.is_empty() {
                if last {
                    output.push_str(&self.decode_selected(&[], true));
                } else {
                    self.detection = Some(detection);
                }
                return output;
            }
        }
        let take = remaining
            .len()
            .min(HTML_DETECTION_SAMPLE_BYTES - detection.sample.len());
        detection.sample.extend_from_slice(&remaining[..take]);
        remaining = &remaining[take..];
        if detection.sample.len() < HTML_DETECTION_SAMPLE_BYTES && !last {
            self.detection = Some(detection);
            return output;
        }
        detection
            .detector
            .feed(&detection.sample, last && remaining.is_empty());
        let encoding = detection.detector.guess(None, true);
        // All already-published bytes were ASCII without escape sequences.
        // Switching here never reparses a node or reexecutes an early script.
        self.encoding_name = Some(encoding.name());
        self.decoder = Some(encoding.new_decoder_without_bom_handling());
        output.push_str(&self.decode_selected(&detection.sample, last && remaining.is_empty()));
        if !remaining.is_empty() {
            output.push_str(&self.decode_selected(remaining, last));
        }
        output
    }

    fn decode_selected(&mut self, bytes: &[u8], last: bool) -> String {
        // Preserve pending legacy lead bytes across empty transport chunks.
        if bytes.is_empty() && !last {
            return String::new();
        }
        let decoder = self.decoder.as_mut().expect("decoder initialized");
        let mut input = bytes;
        let mut output = String::new();
        let mut scratch = [0_u8; 8192];
        loop {
            let (result, read, written, _) = decoder.decode_to_utf8(input, &mut scratch, last);
            output.push_str(std::str::from_utf8(&scratch[..written]).expect("decoder emits UTF-8"));
            input = &input[read..];
            if result == encoding_rs::CoderResult::InputEmpty {
                return output;
            }
        }
    }
}

pub fn decode_resource_text(
    bytes: &[u8],
    content_type: Option<&str>,
    kind: TextResourceKind,
) -> (String, &'static str) {
    decode_resource_text_with_fallback(bytes, content_type, kind, None)
}

/// HTML subdocuments can inherit an ASCII-compatible encoding from their
/// same-origin parent. Stylesheets use their document or importing sheet encoding
/// as the CSS environment fallback. Other resource kinds do not inherit it.
pub fn decode_resource_text_with_fallback(
    bytes: &[u8],
    content_type: Option<&str>,
    kind: TextResourceKind,
    fallback: Option<&str>,
) -> (String, &'static str) {
    let fallback = resource_fallback(kind, fallback);
    let encoding = select_encoding(bytes, content_type, kind, fallback);
    let bytes = &bytes[resource_bom_length(bytes, kind)..];
    let (decoded, _) = encoding.decode_without_bom_handling(bytes);
    (decoded.into_owned(), encoding.name())
}

/// HTML's inherited fallback excludes UTF-16 and replacement encodings.
/// Returning None is significant: native detection may then select an encoding.
pub fn document_fallback_encoding(label: Option<&str>) -> Option<&'static str> {
    label
        .and_then(|label| encoding_rs::Encoding::for_label(label.as_bytes()))
        .filter(|encoding| encoding.is_ascii_compatible() && *encoding != encoding_rs::REPLACEMENT)
        .map(encoding_rs::Encoding::name)
}

fn resource_fallback(
    kind: TextResourceKind,
    label: Option<&str>,
) -> Option<&'static encoding_rs::Encoding> {
    label
        .and_then(|label| encoding_rs::Encoding::for_label(label.as_bytes()))
        .filter(|encoding| {
            matches!(kind, TextResourceKind::Css | TextResourceKind::JavaScript)
                || encoding.is_ascii_compatible()
        })
}

fn resource_bom_length(bytes: &[u8], kind: TextResourceKind) -> usize {
    if kind == TextResourceKind::ModuleJavaScript {
        return if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
            3
        } else {
            0
        };
    }
    encoding_rs::Encoding::for_bom(bytes).map_or(0, |(_, length)| length)
}

fn select_encoding(
    bytes: &[u8],
    content_type: Option<&str>,
    kind: TextResourceKind,
    fallback: Option<&'static encoding_rs::Encoding>,
) -> &'static encoding_rs::Encoding {
    if kind == TextResourceKind::ModuleJavaScript {
        return encoding_rs::UTF_8;
    }
    let bom = encoding_rs::Encoding::for_bom(bytes).map(|(encoding, _)| encoding);
    let mime = content_type.and_then(oryn_common::mime_type::MimeType::parse);
    let header = mime.as_ref().and_then(|mime| mime.parameter("charset"));
    let xml_encoding = matches!(kind, TextResourceKind::Xml)
        .then(|| xml_decl_encoding(bytes))
        .flatten();
    let html_meta = matches!(kind, TextResourceKind::Html)
        .then(|| html_meta_charset(bytes))
        .flatten();
    let css_encoding = matches!(kind, TextResourceKind::Css)
        .then(|| css_decl_encoding(bytes))
        .flatten();
    bom.or_else(|| header.and_then(|label| encoding_rs::Encoding::for_label(label.as_bytes())))
        .or(css_encoding)
        .or(xml_encoding)
        .or(html_meta)
        .or_else(|| {
            matches!(kind, TextResourceKind::Html | TextResourceKind::PlainText)
                .then_some(fallback.unwrap_or(encoding_rs::WINDOWS_1252))
        })
        .or_else(|| {
            matches!(kind, TextResourceKind::Css).then_some(fallback.unwrap_or(encoding_rs::UTF_8))
        })
        .or_else(|| {
            matches!(kind, TextResourceKind::JavaScript)
                .then_some(fallback.unwrap_or(encoding_rs::UTF_8))
        })
        .unwrap_or(encoding_rs::UTF_8)
}

// CSS Syntax's byte-level encoding declaration is deliberately stricter than
// an at-rule: exact lowercase spelling, double quotes, and no leading trivia.
fn css_decl_encoding(bytes: &[u8]) -> Option<&'static encoding_rs::Encoding> {
    let window = &bytes[..bytes.len().min(1024)];
    let rest = window.strip_prefix(b"@charset \"")?;
    let end = rest.iter().position(|byte| *byte == b'"')?;
    if rest.get(end + 1) != Some(&b';') {
        return None;
    }
    let encoding = encoding_rs::Encoding::for_label(&rest[..end])?;
    Some(
        if encoding == encoding_rs::UTF_16LE || encoding == encoding_rs::UTF_16BE {
            encoding_rs::UTF_8
        } else {
            encoding
        },
    )
}

// XML declarations are ASCII-compatible until the encoding name. UTF-16
// signatures must be recognized before attempting that ASCII prescan.
fn xml_decl_encoding(bytes: &[u8]) -> Option<&'static encoding_rs::Encoding> {
    oryn_common::xml_encoding::declaration_encoding(bytes)
}

fn html_content_charset(value: &str) -> Option<&'static encoding_rs::Encoding> {
    let lower = value.to_ascii_lowercase();
    let mut remaining = lower.as_str();
    while let Some(index) = remaining.find("charset") {
        remaining = remaining[index + 7..].trim_start_matches(|ch: char| ch.is_ascii_whitespace());
        let Some(value) = remaining.strip_prefix('=') else {
            continue;
        };
        let value = value.trim_start_matches(|ch: char| ch.is_ascii_whitespace());
        let label = if let Some(quote) = value.chars().next().filter(|ch| matches!(ch, '\'' | '"'))
        {
            let value = &value[1..];
            &value[..value.find(quote).unwrap_or(value.len())]
        } else {
            &value[..value
                .find(|ch: char| ch.is_ascii_whitespace() || ch == ';')
                .unwrap_or(value.len())]
        };
        return encoding_rs::Encoding::for_label(label.as_bytes());
    }
    None
}

fn html_meta_charset(bytes: &[u8]) -> Option<&'static encoding_rs::Encoding> {
    let bytes = &bytes[..bytes.len().min(1024)];
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes[cursor..].starts_with(b"<!--") {
            let end = bytes[cursor + 4..]
                .windows(3)
                .position(|part| part == b"-->")?;
            cursor += 4 + end + 3;
            continue;
        }
        if bytes[cursor] != b'<' {
            cursor += 1;
            continue;
        }
        cursor += 1;
        // A stray less-than sign is text, not a tag whose attributes can
        // consume a later real meta element. Declarations and end tags keep
        // their separate skip path below.
        if !bytes
            .get(cursor)
            .is_some_and(|byte| byte.is_ascii_alphabetic() || matches!(byte, b'!' | b'?' | b'/'))
        {
            continue;
        }
        let start = cursor;
        while cursor < bytes.len() && bytes[cursor].is_ascii_alphanumeric() {
            cursor += 1;
        }
        let is_meta = bytes[start..cursor].eq_ignore_ascii_case(b"meta")
            && bytes
                .get(cursor)
                .is_some_and(|byte| byte.is_ascii_whitespace() || matches!(byte, b'/' | b'>'));
        let mut attributes = Vec::new();
        let mut seen_attributes = std::collections::BTreeSet::new();
        while cursor < bytes.len() && bytes[cursor] != b'>' {
            if bytes[cursor].is_ascii_whitespace() || bytes[cursor] == b'/' {
                cursor += 1;
                continue;
            }
            let start = cursor;
            while cursor < bytes.len()
                && !bytes[cursor].is_ascii_whitespace()
                && !matches!(bytes[cursor], b'=' | b'>' | b'/')
            {
                cursor += 1;
            }
            let name = String::from_utf8_lossy(&bytes[start..cursor]).to_ascii_lowercase();
            while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            let mut value = String::new();
            if cursor < bytes.len() && bytes[cursor] == b'=' {
                cursor += 1;
                while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                    cursor += 1;
                }
                let quote = bytes
                    .get(cursor)
                    .copied()
                    .filter(|byte| matches!(byte, b'\'' | b'"'));
                if quote.is_some() {
                    cursor += 1;
                }
                let start = cursor;
                while cursor < bytes.len()
                    && match quote {
                        Some(quote) => bytes[cursor] != quote,
                        None => !bytes[cursor].is_ascii_whitespace() && bytes[cursor] != b'>',
                    }
                {
                    cursor += 1;
                }
                value = String::from_utf8_lossy(&bytes[start..cursor]).into_owned();
                if quote.is_some() && cursor < bytes.len() {
                    cursor += 1;
                }
            }
            if is_meta && !name.is_empty() && seen_attributes.insert(name.clone()) {
                attributes.push((name, value));
            }
        }
        cursor = cursor.saturating_add(1);
        if !is_meta {
            continue;
        }
        // Prescan attributes are processed in source order. A later charset
        // attribute replaces an earlier content-derived encoding even when its
        // label is invalid; a later content attribute only fills an empty one.
        let mut charset = None;
        let mut need_pragma = None;
        let mut got_pragma = false;
        for (name, value) in attributes {
            match name.as_str() {
                "http-equiv" => {
                    if value.eq_ignore_ascii_case("content-type") {
                        got_pragma = true;
                    }
                }
                "charset" => {
                    charset = encoding_rs::Encoding::for_label(value.as_bytes());
                    need_pragma = Some(false);
                }
                "content" if charset.is_none() => {
                    if let Some(encoding) = html_content_charset(&value) {
                        charset = Some(encoding);
                        need_pragma = Some(true);
                    }
                }
                _ => {}
            }
        }
        if need_pragma.is_none() || (need_pragma == Some(true) && !got_pragma) {
            continue;
        }
        if let Some(encoding) = charset {
            return Some(
                if encoding == encoding_rs::UTF_16LE || encoding == encoding_rs::UTF_16BE {
                    encoding_rs::UTF_8
                } else if encoding == encoding_rs::X_USER_DEFINED {
                    encoding_rs::WINDOWS_1252
                } else {
                    encoding
                },
            );
        }
    }
    None
}

/// Decode a content-coded response incrementally. The source is counted before
/// decoding and the output is counted independently, preventing compressed
/// payloads from bypassing either browser limit.
pub fn decode_content_stream(
    source: BodyStream,
    encoding: ContentEncoding,
    compressed_limit: usize,
    decoded_limit: usize,
) -> BodyStream {
    if encoding == ContentEncoding::Identity {
        return limit_identity_stream(source, compressed_limit, decoded_limit);
    }
    let counted: Pin<Box<dyn Stream<Item = Result<Bytes, std::io::Error>> + Send>> = Box::pin(
        async_stream::try_stream! {
            let mut compressed = 0_usize;
            futures_util::pin_mut!(source);
            while let Some(chunk) = source.next().await {
                let chunk = chunk.map_err(|error| std::io::Error::other(error.to_string()))?;
                compressed = compressed.saturating_add(chunk.len());
                if compressed > compressed_limit {
                    Err(std::io::Error::other(format!("oryn-compressed-limit:{compressed_limit}")))?;
                }
                yield chunk;
            }
        },
    );
    let reader = StreamReader::new(counted);
    match encoding {
        ContentEncoding::Identity => unreachable!("identity returned before decoder creation"),
        ContentEncoding::Gzip => limit_reader(
            async_compression::tokio::bufread::GzipDecoder::new(BufReader::new(reader)),
            decoded_limit,
        ),
        ContentEncoding::Deflate => limit_reader(
            async_compression::tokio::bufread::ZlibDecoder::new(BufReader::new(reader)),
            decoded_limit,
        ),
        ContentEncoding::Brotli => limit_reader(
            async_compression::tokio::bufread::BrotliDecoder::new(BufReader::new(reader)),
            decoded_limit,
        ),
        ContentEncoding::Zstd => limit_reader(
            async_compression::tokio::bufread::ZstdDecoder::new(BufReader::new(reader)),
            decoded_limit,
        ),
    }
}

fn limit_identity_stream(
    mut source: BodyStream,
    compressed_limit: usize,
    decoded_limit: usize,
) -> BodyStream {
    Box::pin(async_stream::try_stream! {
        let mut total = 0_usize;
        while let Some(chunk) = source.next().await {
            let chunk = chunk?;
            total = total.saturating_add(chunk.len());
            if total > compressed_limit {
                Err(TransportError::CompressedLimit { limit: compressed_limit })?;
            }
            if total > decoded_limit {
                Err(TransportError::DecodedLimit { limit: decoded_limit })?;
            }
            yield chunk;
        }
    })
}

fn limit_reader<R>(reader: R, decoded_limit: usize) -> BodyStream
where
    R: AsyncRead + Send + Unpin + 'static,
{
    let mut stream = ReaderStream::new(reader);
    Box::pin(async_stream::try_stream! {
        let mut decoded = 0_usize;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| {
                let message = error.to_string();
                message
                    .strip_prefix("oryn-compressed-limit:")
                    .and_then(|value| value.parse().ok())
                    .map(|limit| TransportError::CompressedLimit { limit })
                    .unwrap_or(TransportError::Failed(message))
            })?;
            decoded = decoded.saturating_add(chunk.len());
            if decoded > decoded_limit {
                Err(TransportError::DecodedLimit { limit: decoded_limit })?;
            }
            yield chunk;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn explicit_trust_configuration_discards_clients_built_under_prior_trust() {
        let transport = RustlsTransport::new(Duration::from_secs(2), 1024).unwrap();
        transport
            .clients
            .lock()
            .await
            .insert("previous-trust".into(), reqwest::Client::new());
        let previous_clients = transport.clients.clone();
        let explicit = transport.with_trusted_roots(&[]).unwrap();
        assert!(explicit.explicit_roots.as_ref().unwrap().is_empty());
        assert!(explicit.clients.lock().await.is_empty());
        assert!(!Arc::ptr_eq(&previous_clients, &explicit.clients));
    }

    #[test]
    fn explicit_trust_rejects_invalid_and_unbounded_ca_material() {
        for roots in [
            vec![b"not a PEM certificate".to_vec()],
            vec![b"-----BEGIN CERTIFICATE-----\nAA==\n-----END CERTIFICATE-----\n".to_vec()],
            vec![Vec::new()],
            vec![vec![]; 17],
            vec![vec![0; 1024 * 1024 + 1]],
        ] {
            assert!(
                RustlsTransport::new(Duration::from_secs(2), 1024)
                    .unwrap()
                    .with_trusted_roots(&roots)
                    .is_err()
            );
        }
    }

    use futures_util::stream;
    use std::io::{Read as _, Write as _};

    #[test]
    fn incremental_transport_charset_and_xml_publish_before_eof() {
        let mut html =
            IncrementalTextDecoder::new(Some("text/html; charset=utf-8"), TextResourceKind::Html);
        assert_eq!(
            html.push(b"<script>early()</script>"),
            "<script>early()</script>"
        );
        let mut xml = IncrementalTextDecoder::new(Some("application/xml"), TextResourceKind::Xml);
        assert!(
            xml.push(b"<?xml version='1.0' encoding='windows-1252'")
                .is_empty()
        );
        assert_eq!(
            xml.push(b"?><a>\x80"),
            "<?xml version='1.0' encoding='windows-1252'?><a>€"
        );
        assert_eq!(xml.encoding_name(), Some("windows-1252"));
    }
    #[test]
    fn invalid_or_quoted_transport_charset_does_not_skip_html_prescan() {
        for header in [
            "text/html; charset=invalid",
            r#"text/html; note="charset=utf-8""#,
        ] {
            let mut decoder = IncrementalTextDecoder::new(Some(header), TextResourceKind::Html);
            assert!(decoder.push(b"<html>").is_empty());
            assert!(decoder.push(b"<meta charset=utf-8>\xc3\xa9").is_empty());
            assert_eq!(decoder.finish(), "<html><meta charset=utf-8>é");
            assert_eq!(decoder.encoding_name(), Some("UTF-8"));
        }
    }

    #[test]
    fn xml_encoding_declarations_require_xml_whitespace_and_encoding_names() {
        for source in [
            "<?xml\u{a0}encoding='windows-1252'?><a/>",
            "<?xml\u{c}encoding='windows-1252'?><a/>",
            "<?xml version='1.0'\u{a0}encoding='windows-1252'?><a/>",
            "<?xml version='1.0'encoding='windows-1252'?><a/>",
            "<?xml version='1.0' encoding=' windows-1252 '?><a/>",
            "<?xml-stylesheet encoding='windows-1252'?><a/>",
        ] {
            assert_eq!(xml_decl_encoding(source.as_bytes()), None, "{source}");
        }
        for space in [" ", "\t", "\r", "\n"] {
            let source = format!(
                "<?xml{space}version='1.0'{space}encoding{space}={space}'windows-1252'?><a/>"
            );
            assert_eq!(
                xml_decl_encoding(source.as_bytes()),
                Some(encoding_rs::WINDOWS_1252)
            );
        }
    }

    #[test]
    fn xml_decoding_uses_declarations_and_transport_precedence() {
        let bytes = b"<?xml version='1.0' encoding = 'windows-1252'?><a>\x80</a>";
        let (text, encoding) = decode_resource_text(bytes, Some("text/xml"), TextResourceKind::Xml);
        assert_eq!(encoding, "windows-1252");
        assert!(text.contains('€'));
        let (_, encoding) = decode_resource_text(
            bytes,
            Some("text/xml; charset=utf-8"),
            TextResourceKind::Xml,
        );
        assert_eq!(encoding, "UTF-8");
        let source = "<?xml version='1.0'?><a>水</a>";
        for big_endian in [false, true] {
            let bytes: Vec<u8> = source
                .encode_utf16()
                .flat_map(|unit| {
                    if big_endian {
                        unit.to_be_bytes()
                    } else {
                        unit.to_le_bytes()
                    }
                })
                .collect();
            assert_eq!(
                decode_resource_text(&bytes, None, TextResourceKind::Xml).0,
                source
            );
            for split in 0..=bytes.len() {
                let mut decoder = IncrementalTextDecoder::new(None, TextResourceKind::Xml);
                let text = decoder.push(&bytes[..split])
                    + &decoder.push(&bytes[split..])
                    + &decoder.finish();
                assert_eq!(text, source);
            }
        }
        assert_eq!(
            decode_resource_text(
                b"<a charset='windows-1252'>ok</a>",
                None,
                TextResourceKind::Xml
            )
            .1,
            "UTF-8"
        );
    }

    #[test]
    fn incremental_decoding_handles_large_chunks_and_prescan_boundary_sequences() {
        let mut bytes = b"<meta charset=utf-8>".to_vec();
        bytes.resize(1023, b' ');
        bytes.extend_from_slice("水".repeat(20000).as_bytes());
        let expected = String::from_utf8(bytes.clone()).unwrap();
        for split in [0, 1, 1023, 1024, 1025, 8192, bytes.len()] {
            let mut decoder = IncrementalTextDecoder::new(None, TextResourceKind::Html);
            let actual =
                decoder.push(&bytes[..split]) + &decoder.push(&bytes[split..]) + &decoder.finish();
            assert_eq!(actual, expected, "split at {split}");
            assert!(decoder.prescan.is_empty());
            assert_eq!(decoder.encoding_name(), Some("UTF-8"));
        }
    }

    #[test]
    fn incremental_decoding_preserves_legacy_leads_across_empty_network_chunks() {
        let mut decoder = IncrementalTextDecoder::new(
            Some("text/html;charset=shift_jis"),
            TextResourceKind::Html,
        );
        assert_eq!(decoder.push(&vec![b'A'; 1024]), "A".repeat(1024));
        assert_eq!(decoder.push(&[0x82]), "");
        assert_eq!(decoder.push(&[]), "");
        assert_eq!(decoder.push(&[0xa0]), "あ");
        assert_eq!(decoder.finish(), "");
    }

    #[test]
    fn document_charset_uses_browser_mime_parameter_parsing() {
        for (header, expected) in [
            (
                r#"text/html;note=";charset=windows-1251";charset=utf-8"#,
                "UTF-8",
            ),
            (r#"text/html;charset="windows-1252"junk"#, "windows-1252"),
            (r#"text/html;charset="windows\-1252""#, "windows-1252"),
            ("text/html;charset=;charset=utf-8", "UTF-8"),
            ("text/html;charset=invalid;charset=utf-8", "windows-1252"),
            ("text/html;charset='utf-8'", "windows-1252"),
            ("invalid;charset=utf-8", "windows-1252"),
        ] {
            assert_eq!(
                decode_resource_text(b"body", Some(header), TextResourceKind::Html).1,
                expected,
                "{header}"
            );
            let mut incremental = IncrementalTextDecoder::new(Some(header), TextResourceKind::Html);
            incremental.push(b"body");
            incremental.finish();
            assert_eq!(incremental.encoding_name(), Some(expected), "{header}");
        }
    }

    #[test]
    fn module_text_is_utf8_at_every_chunk_boundary() {
        for (bytes, expected) in [
            (b"\xef\xbb\xbf\xef\xbb\xbfA".as_slice(), "\u{feff}A"),
            (b"\xff\xfeA\0".as_slice(), "��A\0"),
            (
                "export const name='é';".as_bytes(),
                "export const name='é';",
            ),
        ] {
            let header = Some("text/javascript;charset=windows-1252");
            assert_eq!(
                decode_resource_text(bytes, header, TextResourceKind::ModuleJavaScript),
                (expected.into(), "UTF-8")
            );
            for split in 0..=bytes.len() {
                let mut decoder =
                    IncrementalTextDecoder::new(header, TextResourceKind::ModuleJavaScript);
                let output = decoder.push(&bytes[..split])
                    + &decoder.push(&bytes[split..])
                    + &decoder.finish();
                assert_eq!(output, expected, "split {split}");
            }
        }
    }

    #[test]
    fn linked_stylesheet_mime_requires_css_except_cors_same_origin_quirks() {
        for mime in [
            None,
            Some("text/html"),
            Some("invalid"),
            Some("text/css"),
            Some("text/html, text/css"),
            Some("text/css, text/plain"),
        ] {
            for quirks in [false, true] {
                for same in [false, true] {
                    for nosniff in [false, true] {
                        let mut headers = mime
                            .map(|value| vec![("content-type".into(), value.into())])
                            .unwrap_or_default();
                        if nosniff {
                            headers.push(("x-content-type-options".into(), "nosniff".into()));
                        }
                        let css = matches!(mime, Some("text/css" | "text/html, text/css"));
                        assert_eq!(
                            stylesheet_response_allowed(&headers, quirks, same),
                            css || quirks && same && !nosniff,
                            "{mime:?}, quirks={quirks}, same={same}, nosniff={nosniff}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn stylesheet_nosniff_requires_a_css_mime_essence() {
        for (mime, allowed) in [
            (None, false),
            (Some("text/css"), true),
            (Some("TEXT/CSS;charset=utf-8"), true),
            (Some("text/plain"), false),
            (Some("application/css"), false),
            (Some("text/css/extra"), false),
        ] {
            let mut headers = Vec::new();
            if let Some(mime) = mime {
                headers.push(("Content-Type".into(), mime.into()));
            }
            assert!(stylesheet_response_mime_allowed(&headers));
            headers.push(("X-Content-Type-Options".into(), "NoSniff".into()));
            assert_eq!(
                stylesheet_response_mime_allowed(&headers),
                allowed,
                "{headers:?}"
            );
        }
    }

    #[test]
    fn script_mime_blocking_respects_nosniff_and_module_requirements() {
        for (mime, options, classic, module) in [
            (None, None, true, false),
            (None, Some("nosniff"), false, false),
            (Some("text/javascript"), Some("nosniff"), true, true),
            (
                Some("text/jscript;charset=utf-8"),
                Some("NoSnIfF, anything"),
                true,
                true,
            ),
            (Some("text/plain"), None, true, false),
            (Some("text/plain"), Some("nosniff"), false, false),
            (Some("text/plain"), Some(" nosniff "), false, false),
            (Some("text/plain"), Some("other,nosniff"), true, false),
            (Some("text/plain"), Some("nosniff;other"), true, false),
            (Some("image/svg+xml"), None, false, false),
            (Some("audio/wav"), None, false, false),
            (Some("video/mp4"), None, false, false),
            (Some("TEXT/CSV; charset=utf-8"), None, false, false),
        ] {
            let mut headers = Vec::new();
            if let Some(mime) = mime {
                headers.push(("Content-Type".into(), mime.into()));
            }
            if let Some(options) = options {
                headers.push(("X-Content-Type-Options".into(), options.into()));
            }
            assert_eq!(
                script_response_mime_allowed(&headers, false),
                classic,
                "{headers:?}"
            );
            assert_eq!(
                script_response_mime_allowed(&headers, true),
                module,
                "{headers:?}"
            );
        }
    }

    #[test]
    fn charset_decoding_is_explicit() {
        let (decoded, encoding) = decode_text(
            &[0x63, 0x61, 0x66, 0xe9],
            Some("text/plain; charset=windows-1252"),
        );
        assert_eq!(decoded, "café");
        assert_eq!(encoding, "windows-1252");
    }

    #[test]
    fn html_prescan_respects_attribute_boundaries_pragma_and_fallback() {
        for (source, expected) in [
            ("<meta charset='iso-8859-8 visual'>", "UTF-8"),
            (
                "<!-- <meta charset=windows-1251> --><meta charset=utf-8>",
                "UTF-8",
            ),
            ("<div title=\"charset=windows-1251\">", "UTF-8"),
            ("<meta! charset=windows-1251><meta charset=utf-8>", "UTF-8"),
            (
                "<meta:foreign charset=windows-1251><meta charset=utf-8>",
                "UTF-8",
            ),
            (
                "<meta-custom charset=windows-1251><meta charset=utf-8>",
                "UTF-8",
            ),
            (
                "<meta\u{b} charset=windows-1251><meta charset=utf-8>",
                "UTF-8",
            ),
            ("<meta/charset=windows-1251>", "windows-1251"),
            ("<meta\u{c}charset=windows-1251>", "windows-1251"),
            ("<meta content='text/html;charset=windows-1251'>", "UTF-8"),
            (
                "<meta content='text/html; charset=windows-1251' http-equiv=Content-Type>",
                "windows-1251",
            ),
            (
                "<meta charset=unsupported><meta charset=windows-1251>",
                "windows-1251",
            ),
            ("<META charset = ' UTF-16LE '>", "UTF-8"),
            ("<meta charset=x-user-defined>", "windows-1252"),
            ("<meta charset=utf-8 charset=windows-1251>", "UTF-8"),
        ] {
            let (_, encoding) = decode_resource_text_with_fallback(
                source.as_bytes(),
                Some("text/html"),
                TextResourceKind::Html,
                Some("UTF-8"),
            );
            assert_eq!(encoding, expected, "{source}");
        }
        assert_eq!(
            decode_resource_text(b"\xc2\x80", None, TextResourceKind::Html),
            ("Â€".into(), "windows-1252")
        );
        assert_eq!(
            decode_resource_text_with_fallback(
                b"\xca",
                None,
                TextResourceKind::Html,
                Some("windows-1251")
            ),
            ("К".into(), "windows-1251")
        );
        assert_eq!(
            decode_resource_text_with_fallback(
                b"x",
                None,
                TextResourceKind::Xml,
                Some("windows-1251")
            )
            .1,
            "UTF-8"
        );
        assert_eq!(
            decode_resource_text_with_fallback(
                b"x",
                None,
                TextResourceKind::Html,
                Some("UTF-16LE")
            )
            .1,
            "windows-1252"
        );
    }

    #[test]
    fn html_prescan_stray_less_than_does_not_hide_a_later_meta_element() {
        for prefix in ["< ", "<3 ", "<<", "<\u{0} "] {
            let source = format!("{prefix}<meta charset=windows-1251>");
            assert_eq!(
                html_meta_charset(source.as_bytes()),
                Some(encoding_rs::WINDOWS_1251)
            );
        }
        for source in [
            "<!-- <meta charset=windows-1251> -->",
            "<?ignored <meta charset=windows-1251>>",
            "</meta charset=windows-1251>",
            "<div data-value='<meta charset=windows-1251>'>",
        ] {
            assert_eq!(html_meta_charset(source.as_bytes()), None, "{source}");
        }
    }

    #[test]
    fn html_prescan_keeps_attribute_order_invalid_labels_and_pragma_requirements() {
        for (meta, encoding, tail) in [
            (
                "<meta content='text/html;charset=windows-1251' charset=invalid http-equiv=content-type>",
                "UTF-8",
                "�",
            ),
            (
                "<meta charset=invalid content='text/html;charset=windows-1251' http-equiv=content-type>",
                "windows-1251",
                "К",
            ),
            (
                "<meta charset=invalid content='text/html;charset=windows-1251'>",
                "UTF-8",
                "�",
            ),
            (
                "<meta content='text/html;charset=windows-1251' charset=windows-1252>",
                "windows-1252",
                "Ê",
            ),
            (
                "<meta charset=invalid charset=windows-1251><meta charset=utf-8>",
                "UTF-8",
                "�",
            ),
            (
                "<meta content='text/html;charset=windows-1251' http-equiv=other http-equiv=content-type>",
                "UTF-8",
                "�",
            ),
        ] {
            let mut bytes = meta.as_bytes().to_vec();
            bytes.push(0xca);
            let expected = format!("{meta}{tail}");
            assert_eq!(
                decode_resource_text_with_fallback(
                    &bytes,
                    Some("text/html"),
                    TextResourceKind::Html,
                    Some("UTF-8")
                ),
                (expected.clone(), encoding)
            );
            for split in 0..=bytes.len() {
                let mut decoder = IncrementalTextDecoder::with_fallback(
                    Some("text/html"),
                    TextResourceKind::Html,
                    Some("UTF-8"),
                );
                let mut text = decoder.push(&bytes[..split]);
                text.push_str(&decoder.push(&bytes[split..]));
                text.push_str(&decoder.finish());
                assert_eq!(text, expected, "{meta} at {split}");
                assert_eq!(decoder.encoding_name(), Some(encoding));
            }
        }
    }

    #[test]
    fn classic_script_encoding_preserves_fallback_below_transport_and_bom() {
        for (bytes, header, kind, expected) in [
            (&b"\xca"[..], None, TextResourceKind::JavaScript, "К"),
            (
                &b"\xca"[..],
                Some("text/javascript;charset=windows-1252"),
                TextResourceKind::JavaScript,
                "Ê",
            ),
            (
                &b"\xca"[..],
                Some("text/javascript;charset=invalid-label"),
                TextResourceKind::JavaScript,
                "К",
            ),
            (
                &b"\xef\xbb\xbf\xd0\x9a"[..],
                Some("text/javascript;charset=windows-1252"),
                TextResourceKind::JavaScript,
                "К",
            ),
            (&b"\xca"[..], None, TextResourceKind::ModuleJavaScript, "�"),
        ] {
            assert_eq!(
                decode_resource_text_with_fallback(bytes, header, kind, Some("windows-1251")).0,
                expected
            );
            for split in 0..=bytes.len() {
                let mut decoder =
                    IncrementalTextDecoder::with_fallback(header, kind, Some("windows-1251"));
                let mut text = decoder.push(&bytes[..split]);
                text.push_str(&decoder.push(&bytes[split..]));
                text.push_str(&decoder.finish());
                assert_eq!(text, expected);
            }
        }
    }

    #[test]
    fn streamed_html_fallback_rejects_non_ascii_compatible_parent_encodings() {
        let bytes = b"<p>\xe9</p>";
        for fallback in [
            "UTF-16LE",
            "UTF-16BE",
            "replacement",
            "windows-1252",
            "windows-1251",
            "utf-8",
        ] {
            let expected = match fallback {
                "windows-1251" => "<p>й</p>",
                "utf-8" => "<p>�</p>",
                _ => "<p>é</p>",
            };
            let (buffered, encoding) = decode_resource_text_with_fallback(
                bytes,
                Some("text/html"),
                TextResourceKind::Html,
                Some(fallback),
            );
            assert_eq!(buffered, expected);
            for split in 0..=bytes.len() {
                let mut decoder = IncrementalTextDecoder::with_fallback(
                    Some("text/html"),
                    TextResourceKind::Html,
                    Some(fallback),
                );
                let mut actual = decoder.push(&bytes[..split]);
                actual.push_str(&decoder.push(&[]));
                actual.push_str(&decoder.push(&bytes[split..]));
                actual.push_str(&decoder.finish());
                assert_eq!(actual, expected, "{fallback} split {split}");
                assert_eq!(decoder.encoding_name(), Some(encoding));
            }
        }
    }

    #[test]
    fn stylesheet_environment_encoding_is_a_fallback_below_declarations() {
        assert_eq!(
            decode_resource_text_with_fallback(
                b"\xe9",
                None,
                TextResourceKind::Css,
                Some("windows-1252")
            ),
            ("é".into(), "windows-1252")
        );
        assert_eq!(
            decode_resource_text_with_fallback(
                b"@charset \"windows-1251\";\xe9",
                None,
                TextResourceKind::Css,
                Some("windows-1252")
            )
            .0,
            "@charset \"windows-1251\";й"
        );
        assert_eq!(
            decode_resource_text_with_fallback(
                b"\xef\xbb\xbfA",
                None,
                TextResourceKind::Css,
                Some("windows-1252")
            ),
            ("A".into(), "UTF-8")
        );
    }

    #[test]
    fn css_charset_selection_is_strict_and_respects_header_and_bom_precedence() {
        let bytes = b"@charset \"windows-1252\";p::before{content:'\xe9'}";
        let (text, encoding) = decode_resource_text(bytes, None, TextResourceKind::Css);
        assert_eq!(encoding, "windows-1252");
        assert!(text.contains("é"));
        for split in 0..=bytes.len() {
            let mut decoder = IncrementalTextDecoder::new(None, TextResourceKind::Css);
            let actual =
                decoder.push(&bytes[..split]) + &decoder.push(&bytes[split..]) + &decoder.finish();
            assert_eq!(actual, text);
            assert_eq!(decoder.encoding_name(), Some("windows-1252"));
        }
        assert_eq!(
            decode_resource_text(bytes, Some("text/css;charset=utf-8"), TextResourceKind::Css).1,
            "UTF-8"
        );
        let mut bom = vec![0xef, 0xbb, 0xbf];
        bom.extend_from_slice(bytes);
        assert_eq!(
            decode_resource_text(
                &bom,
                Some("text/css;charset=windows-1251"),
                TextResourceKind::Css
            )
            .1,
            "UTF-8"
        );
        for declaration in [
            " @charset \"windows-1252\";",
            "@CHARSET \"windows-1252\";",
            "@charset 'windows-1252';",
            "@charset  \"windows-1252\";",
            "@charset \"windows-1252\" ;",
            "@charset \"utf-16le\";",
            "@charset \"utf-16be\";",
        ] {
            assert_eq!(
                decode_resource_text(declaration.as_bytes(), None, TextResourceKind::Css).1,
                "UTF-8",
                "{declaration}"
            );
        }
        let mut late = vec![b' '; 1024];
        late.extend_from_slice(bytes);
        assert_eq!(
            decode_resource_text(&late, None, TextResourceKind::Css).1,
            "UTF-8"
        );
    }

    #[test]
    fn html_prescan_and_bom_follow_browser_precedence() {
        let (decoded, encoding) = decode_resource_text(
            b"<meta charset=windows-1252>caf\xe9",
            Some("text/html"),
            TextResourceKind::Html,
        );
        assert!(decoded.ends_with("café"));
        assert_eq!(encoding, "windows-1252");

        let (decoded, encoding) = decode_resource_text(
            b"\xef\xbb\xbf<meta charset=windows-1252>ok",
            Some("text/html; charset=windows-1252"),
            TextResourceKind::Html,
        );
        assert!(decoded.ends_with("ok"));
        assert_eq!(encoding, "UTF-8");
    }

    #[test]
    fn resource_decoders_remove_exactly_one_bom_at_every_chunk_boundary() {
        for (bytes, expected_encoding) in [
            (b"\xef\xbb\xbf\xef\xbb\xbfA".as_slice(), "UTF-8"),
            (b"\xff\xfe\xff\xfeA\x00".as_slice(), "UTF-16LE"),
            (b"\xfe\xff\xfe\xff\x00A".as_slice(), "UTF-16BE"),
        ] {
            assert_eq!(
                decode_resource_text(bytes, Some("text/html"), TextResourceKind::Html),
                ("\u{feff}A".into(), expected_encoding)
            );
            for boundary in 0..=bytes.len() {
                let mut decoder =
                    IncrementalTextDecoder::new(Some("text/html"), TextResourceKind::Html);
                let mut text = decoder.push(&bytes[..boundary]);
                text.push_str(&decoder.push(&bytes[boundary..]));
                text.push_str(&decoder.finish());
                assert_eq!(text, "\u{feff}A", "{expected_encoding} at {boundary}");
            }
        }
    }

    #[test]
    fn incremental_decoder_preserves_multibyte_and_charset_state_at_every_boundary() {
        let source = "<meta charset=utf-8><p>café 東京</p>".as_bytes();
        for boundary in 0..=source.len() {
            let mut decoder =
                IncrementalTextDecoder::new(Some("text/html"), TextResourceKind::Html);
            let mut output = decoder.push(&source[..boundary]);
            output.push_str(&decoder.push(&source[boundary..]));
            output.push_str(&decoder.finish());
            assert_eq!(
                output,
                String::from_utf8(source.to_vec()).unwrap(),
                "boundary {boundary}"
            );
            assert_eq!(decoder.encoding_name(), Some("UTF-8"));
        }
    }

    #[tokio::test]
    async fn content_stream_enforces_compressed_and_decoded_limits_separately() {
        let source: BodyStream = Box::pin(stream::iter(vec![Ok(Bytes::from_static(b"abcdef"))]));
        let mut decoded = decode_content_stream(source, ContentEncoding::Identity, 6, 5);
        assert!(matches!(
            decoded.next().await,
            Some(Err(TransportError::DecodedLimit { limit: 5 }))
        ));

        let source: BodyStream = Box::pin(stream::iter(vec![Ok(Bytes::from_static(b"abcdef"))]));
        let mut decoded = decode_content_stream(source, ContentEncoding::Identity, 5, 8);
        assert!(matches!(
            decoded.next().await,
            Some(Err(TransportError::CompressedLimit { limit: 5 }))
        ));
    }

    #[tokio::test]
    async fn http1_transport_emits_browser_connection_header() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let request = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = [0_u8; 4096];
            let size = stream.read(&mut bytes).unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
                .unwrap();
            String::from_utf8_lossy(&bytes[..size]).into_owned()
        });
        let transport = RustlsTransport::new(Duration::from_secs(2), 1024).unwrap();
        let response = transport
            .start(HttpRequest {
                dns_timing: None,
                operation_id: 1,
                method: "GET".into(),
                url: format!("http://{address}/headers"),
                headers: Vec::new(),
                body: Vec::new(),
                resolved_addrs: vec![address],
            })
            .await
            .unwrap();
        assert_eq!(response.status, 200);
        assert!(response.timing.connect_start.is_some());
        assert!(response.timing.connect_end >= response.timing.connect_start);
        assert!(response.timing.secure_start.is_none());
        assert_eq!(response.timing.next_hop_protocol, "http/1.1");
        assert!(response.timing.response_start >= response.timing.request_start);
        let request = request.join().unwrap().to_ascii_lowercase();
        assert!(
            request.contains("\r\nconnection: keep-alive\r\n"),
            "request did not carry the HTTP/1 connection header:\n{request}"
        );
    }
}

#[cfg(test)]
mod transport_lifetime_tests {
    use super::*;

    #[test]
    fn dropping_request_ownership_cancels_and_unregisters_it() {
        let pending: PendingTransportOperations = Arc::new(std::sync::Mutex::new(BTreeMap::new()));
        let lease = TransportOperationLease::register(&pending, 7, 0).unwrap();
        let token = lease.operation.cancellation.clone();
        assert_eq!(pending.lock().unwrap().len(), 1);
        drop(lease);
        assert!(pending.lock().unwrap().is_empty());
        assert!(token.is_cancelled());
    }

    #[test]
    fn duplicate_ids_are_rejected_and_old_leases_cannot_remove_replacements() {
        let pending: PendingTransportOperations = Arc::new(std::sync::Mutex::new(BTreeMap::new()));
        let first = TransportOperationLease::register(&pending, 7, 0).unwrap();
        assert!(TransportOperationLease::register(&pending, 7, 0).is_err());
        let removed = pending.lock().unwrap().remove(&7).unwrap();
        removed.cancellation.cancel();
        let replacement = TransportOperationLease::register(&pending, 7, 0).unwrap();
        drop(first);
        assert_eq!(pending.lock().unwrap().len(), 1);
        assert!(!replacement.operation.cancellation.is_cancelled());
        drop(replacement);
        assert!(pending.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn dropping_an_unpolled_body_releases_registration() {
        let pending: PendingTransportOperations = Arc::new(std::sync::Mutex::new(BTreeMap::new()));
        let lease = TransportOperationLease::register(&pending, 8, 0).unwrap();
        let body: BodyStream = Box::pin(async_stream::try_stream! {
            let _lease = lease;
            yield Bytes::from_static(b"body");
        });
        drop(body);
        assert!(pending.lock().unwrap().is_empty());
    }
    #[tokio::test]
    async fn upload_progress_counts_only_polled_chunks() {
        let pending: PendingTransportOperations = Arc::new(std::sync::Mutex::new(BTreeMap::new()));
        let lease = TransportOperationLease::register(&pending, 9, 65_537).unwrap();
        let mut body = tracked_upload_body(Bytes::from(vec![42; 65_537]), lease.operation.clone());
        assert_eq!(
            lease.operation.upload_progress(),
            UploadProgress {
                bytes_consumed: 0,
                total_bytes: 65_537,
                body_complete: false,
            }
        );
        assert_eq!(body.next().await.unwrap().unwrap().len(), 65_536);
        assert_eq!(lease.operation.upload_progress().bytes_consumed, 65_536);
        assert!(!lease.operation.upload_progress().body_complete);
        assert_eq!(body.next().await.unwrap().unwrap().as_ref(), &[42]);
        assert!(lease.operation.upload_progress().body_complete);
        assert!(body.next().await.is_none());
    }

    #[tokio::test]
    async fn cancelled_upload_does_not_count_unconsumed_bytes() {
        let pending: PendingTransportOperations = Arc::new(std::sync::Mutex::new(BTreeMap::new()));
        let lease = TransportOperationLease::register(&pending, 10, 65_537).unwrap();
        let operation = lease.operation.clone();
        let mut body = tracked_upload_body(Bytes::from(vec![42; 65_537]), operation.clone());
        body.next().await.unwrap().unwrap();
        drop(lease);
        assert!(matches!(
            body.next().await,
            Some(Err(TransportError::Cancelled))
        ));
        assert_eq!(operation.upload_progress().bytes_consumed, 65_536);
        assert!(!operation.upload_progress().body_complete);
        assert!(pending.lock().unwrap().is_empty());
    }
}

#[cfg(test)]
mod phase_offset_tests {
    use super::*;
    #[test]
    fn reused_connection_phases_are_not_replayed_into_new_request() {
        let old = std::time::Instant::now();
        let request = old + Duration::from_secs(1);
        assert_eq!(phase_offset(request, Some(old)), None);
        assert_eq!(
            phase_offset(request, Some(request + Duration::from_micros(5))),
            Some(5)
        );
    }
}

#[cfg(test)]
mod html_encoding_replay_tests {
    use super::*;
    #[test]
    fn parser_label_replays_admitted_prefix_once_and_preserves_incremental_tail() {
        let mut bytes = b"<!doctype html><head>".to_vec();
        bytes.resize(1100, b' ');
        bytes.extend_from_slice(b"<meta charset=windows-1251>\xe6");
        let mut decoder = IncrementalTextDecoder::new(Some("text/html"), TextResourceKind::Html);
        assert!(decoder.push(&bytes).ends_with('æ'));
        assert_eq!(
            decoder.html_encoding_confidence(),
            HtmlEncodingConfidence::Tentative
        );
        let replay = decoder
            .restart_html_encoding("windows-1251", &bytes, false)
            .unwrap();
        assert!(replay.ends_with('ж'));
        assert_eq!(decoder.push(&[0xe6]), "ж");
        assert!(
            decoder
                .restart_html_encoding("utf-8", &bytes, false)
                .is_none()
        );
        assert_eq!(decoder.encoding_name(), Some("windows-1251"));
    }
    #[test]
    fn certain_headers_bom_and_author_commit_reject_late_labels() {
        for (header, bytes) in [
            (Some("text/html;charset=utf-8"), b"abcd".as_slice()),
            (None, b"\xef\xbb\xbfabc".as_slice()),
        ] {
            let mut decoder = IncrementalTextDecoder::new(header, TextResourceKind::Html);
            decoder.push(bytes);
            decoder.finish();
            assert_eq!(
                decoder.html_encoding_confidence(),
                HtmlEncodingConfidence::Certain
            );
            assert!(
                decoder
                    .restart_html_encoding("windows-1251", bytes, true)
                    .is_none()
            );
        }
        let mut decoder = IncrementalTextDecoder::new(None, TextResourceKind::Html);
        decoder.push(&vec![b' '; 1024]);
        decoder.commit_html_encoding();
        assert!(
            decoder
                .restart_html_encoding("windows-1251", b"abc", false)
                .is_none()
        );
    }
    #[test]
    fn invalid_label_does_not_commit_and_same_label_does_not_reparse() {
        let mut decoder = IncrementalTextDecoder::new(None, TextResourceKind::Html);
        decoder.push(&vec![b' '; 1024]);
        assert!(
            decoder
                .restart_html_encoding("unknown", b"abc", false)
                .is_none()
        );
        assert_eq!(
            decoder.html_encoding_confidence(),
            HtmlEncodingConfidence::Tentative
        );
        assert!(
            decoder
                .restart_html_encoding("x-user-defined", b"abc", false)
                .is_none()
        );
        assert_eq!(
            decoder.html_encoding_confidence(),
            HtmlEncodingConfidence::Certain
        );
    }
}

#[cfg(test)]
mod html_detection_tests {
    use super::*;
    fn sample(encoding: &'static encoding_rs::Encoding, text: &str) -> Vec<u8> {
        let mut bytes =
            b"<!doctype html><head><script>globalThis.runs=(globalThis.runs||0)+1</script><!--"
                .to_vec();
        bytes.extend(std::iter::repeat_n(b'a', 9500));
        bytes.extend_from_slice(b"--></head><body><p>");
        let (encoded, _, errors) = encoding.encode(text);
        assert!(!errors);
        bytes.extend_from_slice(&encoded);
        bytes.extend_from_slice(b"</p>");
        bytes
    }
    #[test]
    fn late_non_ascii_detection_preserves_ascii_prefix_without_replay() {
        for (encoding, text) in [
            (encoding_rs::IBM866, "Это тест кодировки символов."),
            (encoding_rs::UTF_8, "Encoding text: Ελληνικά 日本語 Русский"),
        ] {
            let bytes = sample(encoding, text);
            for split in [0, 1, 1023, 1024, 9500, bytes.len() - 2, bytes.len()] {
                let mut decoder =
                    IncrementalTextDecoder::new(Some("text/html"), TextResourceKind::Html)
                        .with_html_detection();
                let mut result = decoder.push(&bytes[..split]);
                if !result.is_empty() {
                    decoder.commit_html_encoding();
                }
                result.push_str(&decoder.push(&bytes[split..]));
                result.push_str(&decoder.finish());
                assert!(result.contains(text), "{} at {split}", encoding.name());
                assert_eq!(result.matches("globalThis.runs=").count(), 1);
                assert_eq!(decoder.encoding_name(), Some(encoding.name()));
            }
        }
    }
    #[test]
    fn header_meta_bom_and_inherited_encoding_disable_statistical_guessing() {
        let bytes = "<p>€€ Hello World! €€</p>".as_bytes();
        for (header, fallback, prefix) in [
            (Some("text/html;charset=windows-1252"), None, ""),
            (Some("text/html"), Some("windows-1252"), ""),
            (Some("text/html"), None, "<meta charset=windows-1252>"),
        ] {
            let mut raw = prefix.as_bytes().to_vec();
            raw.extend_from_slice(bytes);
            let mut decoder =
                IncrementalTextDecoder::with_fallback(header, TextResourceKind::Html, fallback)
                    .with_html_detection();
            let mut text = decoder.push(&raw);
            text.push_str(&decoder.finish());
            assert_eq!(decoder.encoding_name(), Some("windows-1252"));
            assert!(!text.contains("€€"));
        }
        let raw = [&[0xef, 0xbb, 0xbf][..], bytes].concat();
        let mut decoder = IncrementalTextDecoder::new(
            Some("text/html;charset=windows-1252"),
            TextResourceKind::Html,
        )
        .with_html_detection();
        let mut text = decoder.push(&raw);
        text.push_str(&decoder.finish());
        assert!(text.contains("€€"));
        assert_eq!(decoder.encoding_name(), Some("UTF-8"));
    }
    #[test]
    fn same_encoding_meta_does_not_discard_pending_detection_bytes() {
        let mut raw = vec![b' '; 1100];
        raw.extend_from_slice(b"<meta charset=windows-1252><p>\xc0</p>");
        let mut decoder = IncrementalTextDecoder::new(Some("text/html"), TextResourceKind::Html)
            .with_html_detection();
        let initial = decoder.push(&raw);
        assert!(!initial.contains('À'));
        assert!(decoder.html_encoding_restart_needed("windows-1252"));
        let replay = decoder
            .restart_html_encoding("windows-1252", &raw, false)
            .unwrap();
        assert!(replay.ends_with("<p>À</p>"));
        assert_eq!(decoder.finish(), "");
    }
    #[test]
    fn detection_sample_boundary_preserves_multibyte_sequences_and_empty_chunks() {
        let text = format!(
            "<head>{}</head><body>{}</body>",
            " ".repeat(1100),
            "Ελληνικά 日本語 Русский ".repeat(80)
        );
        for width in [1, 2, 3, 7, 31, 1023, 1024, 1025] {
            let mut decoder =
                IncrementalTextDecoder::new(Some("text/html"), TextResourceKind::Html)
                    .with_html_detection();
            let mut output = String::new();
            for chunk in text.as_bytes().chunks(width) {
                output.push_str(&decoder.push(chunk));
                output.push_str(&decoder.push(&[]));
            }
            output.push_str(&decoder.finish());
            assert_eq!(output, text, "transport width {width}");
            assert_eq!(decoder.encoding_name(), Some("UTF-8"));
        }
    }
    #[test]
    fn non_navigation_decoder_does_not_guess_and_sample_is_bounded() {
        let bytes = "<p>日本語</p>".as_bytes();
        let mut plain = IncrementalTextDecoder::new(Some("text/html"), TextResourceKind::Html);
        plain.push(bytes);
        plain.finish();
        assert_eq!(plain.encoding_name(), Some("windows-1252"));
        let mut detector = IncrementalTextDecoder::new(Some("text/html"), TextResourceKind::Html)
            .with_html_detection();
        let raw = [
            vec![b'a'; 1024],
            vec![0xc0; HTML_DETECTION_SAMPLE_BYTES - 1],
        ]
        .concat();
        detector.push(&raw);
        assert!(
            detector
                .detection
                .as_ref()
                .is_some_and(|state| state.sample.len() == HTML_DETECTION_SAMPLE_BYTES - 1)
        );
        detector.push(&[0xc0]);
        assert!(detector.detection.is_none());
    }
}

#[cfg(test)]
mod document_kind_encoding_tests {
    use super::*;
    #[test]
    fn plaintext_inherits_ascii_compatible_encoding_without_html_prescan() {
        let bytes = b"<meta charset=windows-1251>\xc0";
        assert_eq!(
            decode_resource_text_with_fallback(
                bytes,
                Some("text/plain"),
                TextResourceKind::PlainText,
                Some("windows-1252")
            ),
            ("<meta charset=windows-1251>À".into(), "windows-1252")
        );
        assert_eq!(
            decode_resource_text_with_fallback(
                &[0xc0],
                Some("text/plain"),
                TextResourceKind::PlainText,
                Some("windows-1251")
            ),
            ("А".into(), "windows-1251")
        );
        assert_eq!(
            decode_resource_text_with_fallback(
                &[0xc0],
                Some("text/plain"),
                TextResourceKind::PlainText,
                Some("UTF-16LE")
            ),
            ("À".into(), "windows-1252")
        );
    }
    #[test]
    fn xml_and_fetch_do_not_inherit_document_or_css_encodings() {
        for kind in [TextResourceKind::Xml, TextResourceKind::Fetch] {
            let (decoded, encoding) =
                decode_resource_text_with_fallback(&[0xc0], None, kind, Some("windows-1251"));
            assert_eq!(encoding, "UTF-8");
            assert_eq!(decoded, "�");
        }
    }
    #[test]
    fn plaintext_header_and_bom_precedence_matches_streamed_output() {
        for (bytes, header, expected, encoding) in [
            (
                vec![0xc0],
                "text/plain;charset=windows-1251",
                "А",
                "windows-1251",
            ),
            (
                vec![0xff, 0xfe, 0x10, 0x04],
                "text/plain;charset=windows-1252",
                "А",
                "UTF-16LE",
            ),
        ] {
            for split in 0..=bytes.len() {
                let mut decoder = IncrementalTextDecoder::with_fallback(
                    Some(header),
                    TextResourceKind::PlainText,
                    Some("windows-1252"),
                )
                .with_html_detection();
                let mut actual = decoder.push(&bytes[..split]);
                actual.push_str(&decoder.push(&bytes[split..]));
                actual.push_str(&decoder.finish());
                assert_eq!(actual, expected);
                assert_eq!(decoder.encoding_name(), Some(encoding));
            }
        }
    }
}

/// Retain only the TLS verifier's typed certificate rejection. Ordinary TLS,
/// DNS, socket and HTTP failures must not acquire certificate authority.
fn request_error(error: reqwest::Error) -> TransportError {
    match certificate_rejection(&error, 32) {
        Some(detail) => TransportError::CertificateRejected(detail),
        None => TransportError::Failed(error.to_string()),
    }
}

fn certificate_rejection(
    error: &(dyn std::error::Error + 'static),
    mut budget: usize,
) -> Option<String> {
    fn visit(error: &(dyn std::error::Error + 'static), budget: &mut usize) -> Option<String> {
        if *budget == 0 {
            return None;
        }
        *budget -= 1;
        if let Some(rustls::Error::InvalidCertificate(cause)) =
            error.downcast_ref::<rustls::Error>()
        {
            return Some(cause.to_string());
        }
        // io::Error's inner error is not guaranteed to appear as its source.
        if let Some(io) = error.downcast_ref::<std::io::Error>() {
            if let Some(inner) = io.get_ref() {
                if let Some(cause) = visit(inner, budget) {
                    return Some(cause);
                }
            }
        }
        error.source().and_then(|source| visit(source, budget))
    }
    visit(error, &mut budget)
}

#[cfg(test)]
mod certificate_error_tests {
    use super::*;
    #[test]
    fn certificate_rejection_preserves_verifier_causes_through_io_wrapping() {
        for cause in [
            rustls::CertificateError::UnknownIssuer,
            rustls::CertificateError::NotValidForName,
        ] {
            let detail = cause.to_string();
            let error = std::io::Error::other(rustls::Error::InvalidCertificate(cause));
            assert_eq!(certificate_rejection(&error, 32), Some(detail));
        }
    }
    #[test]
    fn certificate_rejection_does_not_infer_from_text_or_other_tls_errors() {
        for error in [
            std::io::Error::other("invalid peer certificate: UnknownIssuer"),
            std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                "certificate rejected",
            ),
            std::io::Error::other(rustls::Error::General("invalid certificate".into())),
            std::io::Error::other(rustls::Error::AlertReceived(
                rustls::AlertDescription::BadCertificate,
            )),
        ] {
            assert_eq!(certificate_rejection(&error, 32), None);
        }
    }
}
