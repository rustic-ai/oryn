//! Transport-neutral asynchronous browser networking contracts.

use std::{collections::BTreeMap, pin::Pin, sync::Arc, time::Duration};

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
pub mod fetch;

pub type BodyStream = Pin<Box<dyn Stream<Item = Result<Bytes, TransportError>> + Send>>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpRequest {
    pub operation_id: u64,
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

pub struct HttpResponse {
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
}

/// Reusable asynchronous rustls transport. Redirects and content decoding are
/// disabled here because the browser Fetch layer must inspect every hop and
/// enforce both compressed and decoded byte limits itself.
pub struct RustlsTransport {
    client: reqwest::Client,
    pending: Arc<Mutex<BTreeMap<u64, CancellationToken>>>,
    max_body_bytes: usize,
}

impl RustlsTransport {
    pub fn new(timeout: Duration, max_body_bytes: usize) -> Result<Self, TransportError> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .no_zstd()
            .build()
            .map_err(|error| TransportError::Failed(error.to_string()))?;
        Ok(Self {
            client,
            pending: Arc::new(Mutex::new(BTreeMap::new())),
            max_body_bytes,
        })
    }
}

#[async_trait]
impl HttpTransport for RustlsTransport {
    async fn start(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|error| TransportError::Failed(error.to_string()))?;
        let mut builder = self.client.request(method, &request.url);
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        if !request.body.is_empty() {
            builder = builder.body(request.body);
        }
        let cancellation = CancellationToken::new();
        self.pending
            .lock()
            .await
            .insert(request.operation_id, cancellation.clone());
        let response = tokio::select! {
            _ = cancellation.cancelled() => return Err(TransportError::Cancelled),
            response = builder.send() => response.map_err(|error| TransportError::Failed(error.to_string()))?,
        };
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
        let operation_id = request.operation_id;
        let pending = self.pending.clone();
        let limit = self.max_body_bytes;
        let mut source = response.bytes_stream();
        let body = Box::pin(async_stream::try_stream! {
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
            pending.lock().await.remove(&operation_id);
        });
        Ok(HttpResponse {
            operation_id,
            status,
            headers,
            body,
        })
    }

    async fn cancel(&self, operation_id: u64) -> Result<(), TransportError> {
        let token = self.pending.lock().await.remove(&operation_id);
        if let Some(token) = token {
            token.cancel();
            Ok(())
        } else {
            Err(TransportError::Cancelled)
        }
    }
}

pub fn decode_text(bytes: &[u8], content_type: Option<&str>) -> (String, &'static str) {
    decode_resource_text(bytes, content_type, TextResourceKind::Fetch)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextResourceKind {
    Html,
    Css,
    JavaScript,
    Fetch,
}

pub fn decode_resource_text(
    bytes: &[u8],
    content_type: Option<&str>,
    kind: TextResourceKind,
) -> (String, &'static str) {
    let bom = encoding_rs::Encoding::for_bom(bytes).map(|(encoding, _)| encoding);
    let header = content_type.and_then(|value| {
        value.split(';').find_map(|part| {
            let (name, value) = part.trim().split_once('=')?;
            name.eq_ignore_ascii_case("charset")
                .then(|| value.trim().trim_matches(['\'', '"']))
        })
    });
    let html_meta = matches!(kind, TextResourceKind::Html)
        .then(|| html_meta_charset(bytes))
        .flatten();
    let encoding = bom
        .or_else(|| header.and_then(|label| encoding_rs::Encoding::for_label(label.as_bytes())))
        .or_else(|| {
            html_meta
                .as_deref()
                .and_then(|label| encoding_rs::Encoding::for_label(label.as_bytes()))
        })
        .or_else(|| matches!(kind, TextResourceKind::Css).then_some(encoding_rs::UTF_8))
        .or_else(|| matches!(kind, TextResourceKind::JavaScript).then_some(encoding_rs::UTF_8))
        .unwrap_or(encoding_rs::UTF_8);
    let bytes = encoding_rs::Encoding::for_bom(bytes)
        .map(|(_, length)| &bytes[length..])
        .unwrap_or(bytes);
    let (decoded, _, _) = encoding.decode(bytes);
    (decoded.into_owned(), encoding.name())
}

fn html_meta_charset(bytes: &[u8]) -> Option<String> {
    let prescan = &bytes[..bytes.len().min(1024)];
    let ascii = String::from_utf8_lossy(prescan).to_ascii_lowercase();
    for marker in ["charset=\"", "charset='", "charset="] {
        if let Some(start) = ascii.find(marker) {
            let value = &ascii[start + marker.len()..];
            let end = value
                .find(|character: char| {
                    character == '"'
                        || character == '\''
                        || character.is_ascii_whitespace()
                        || character == '>'
                })
                .unwrap_or(value.len());
            if end > 0 {
                return Some(value[..end].to_string());
            }
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
    use futures_util::stream;

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
}
