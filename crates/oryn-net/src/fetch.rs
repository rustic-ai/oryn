//! Policy-gated asynchronous browser Fetch pipeline.

use std::sync::Arc;

use url::Url;

use crate::{
    BodyStream, ContentEncoding, HttpRequest, HttpResponse, HttpTransport, TransportError,
    decode_content_stream,
};

#[derive(Debug, Clone)]
pub struct FetchLimits {
    pub max_redirects: usize,
    pub max_compressed_body_bytes: usize,
    pub max_decoded_body_bytes: usize,
}

impl Default for FetchLimits {
    fn default() -> Self {
        Self {
            max_redirects: 20,
            max_compressed_body_bytes: 16 * 1024 * 1024,
            max_decoded_body_bytes: 16 * 1024 * 1024,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error("network policy denied request: {0}")]
    RequestDenied(String),
    #[error("network policy denied response headers: {0}")]
    ResponseDenied(String),
    #[error("redirect response did not contain a valid Location header")]
    InvalidRedirect,
    #[error("redirect limit exceeded")]
    TooManyRedirects,
}

pub trait FetchPolicy: Send + Sync {
    fn validate_request(&self, request: &HttpRequest) -> Result<(), String>;
    fn validate_response_headers(
        &self,
        request: &HttpRequest,
        status: u16,
        headers: &[(String, String)],
    ) -> Result<(), String>;
}

pub struct BrowserResponse {
    /// Measured final-hop transport phases; body completion is not inferred.
    pub timing: crate::TransportTiming,
    pub upload: Option<crate::UploadProgress>,
    pub operation_id: u64,
    pub url: String,
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub redirected: bool,
    pub body: BodyStream,
}

pub struct BrowserFetchPipeline<T, P> {
    transport: Arc<T>,
    policy: Arc<P>,
    limits: FetchLimits,
}

impl<T, P> BrowserFetchPipeline<T, P>
where
    T: HttpTransport + 'static,
    P: FetchPolicy + 'static,
{
    pub fn new(transport: Arc<T>, policy: Arc<P>, limits: FetchLimits) -> Self {
        Self {
            transport,
            policy,
            limits,
        }
    }

    pub async fn start(&self, mut request: HttpRequest) -> Result<BrowserResponse, FetchError> {
        for redirect_count in 0..=self.limits.max_redirects {
            self.policy
                .validate_request(&request)
                .map_err(FetchError::RequestDenied)?;
            let HttpResponse {
                timing,
                upload,
                operation_id,
                status,
                headers,
                body,
            } = self.transport.start(request.clone()).await?;
            if let Err(error) = self
                .policy
                .validate_response_headers(&request, status, &headers)
            {
                let _ = self.transport.cancel(operation_id).await;
                return Err(FetchError::ResponseDenied(error));
            }
            if is_redirect_status(status)
                && let Some(location) = header(&headers, "location")
            {
                // Once a redirect is followed, its body is no longer exposed.
                // Cancel before parsing Location so invalid redirects also release it.
                let _ = self.transport.cancel(operation_id).await;
                if redirect_count == self.limits.max_redirects {
                    return Err(FetchError::TooManyRedirects);
                }
                let base = Url::parse(&request.url).map_err(|_| FetchError::InvalidRedirect)?;
                let next = base
                    .join(location)
                    .map_err(|_| FetchError::InvalidRedirect)?;
                rewrite_redirect_request(
                    status,
                    &base,
                    &next,
                    &mut request.method,
                    &mut request.headers,
                    &mut request.body,
                );
                request.url = next.to_string();
                request.resolved_addrs.clear();
                request.dns_timing = None;
                continue;
            }
            let encoding = ContentEncoding::parse(header(&headers, "content-encoding"))?;
            let body = decode_content_stream(
                body,
                encoding,
                self.limits.max_compressed_body_bytes,
                self.limits.max_decoded_body_bytes,
            );
            let redirected = redirect_count != 0;
            return Ok(BrowserResponse {
                timing,
                upload,
                operation_id,
                url: request.url,
                status,
                headers,
                redirected,
                body,
            });
        }
        Err(FetchError::TooManyRedirects)
    }

    pub async fn cancel(&self, operation_id: u64) -> Result<(), FetchError> {
        self.transport.cancel(operation_id).await?;
        Ok(())
    }
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

pub const fn is_redirect_status(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

/// Apply Fetch's HTTP-redirect request mutation to an already resolved Location.
/// Callers retain responsibility for response policy, redirect limits and cancellation.
pub fn rewrite_redirect_request(
    status: u16,
    previous: &Url,
    next: &Url,
    method: &mut String,
    headers: &mut Vec<(String, String)>,
    body: &mut Vec<u8>,
) {
    if previous.origin() != next.origin() {
        headers.retain(|(name, _)| !name.eq_ignore_ascii_case("authorization"));
    }
    if (status == 303 && !matches!(method.as_str(), "GET" | "HEAD"))
        || (matches!(status, 301 | 302) && method.as_str() == "POST")
    {
        *method = "GET".into();
        body.clear();
        headers.retain(|(name, _)| {
            !matches!(
                name.to_ascii_lowercase().as_str(),
                "content-encoding"
                    | "content-language"
                    | "content-location"
                    | "content-type"
                    | "content-length"
            )
        });
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Mutex};

    use async_trait::async_trait;
    use bytes::Bytes;
    use futures_util::{StreamExt, stream};

    use super::*;

    type MockResponse = (u16, Vec<(String, String)>, Vec<u8>);

    #[derive(Default)]
    struct MockTransport {
        upload: Mutex<Option<crate::UploadProgress>>,
        timings: Mutex<BTreeMap<String, crate::TransportTiming>>,
        responses: Mutex<BTreeMap<String, MockResponse>>,
        cancelled: Mutex<Vec<u64>>,
        requests: Mutex<Vec<HttpRequest>>,
    }

    #[async_trait]
    impl HttpTransport for MockTransport {
        async fn start(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
            self.requests.lock().unwrap().push(request.clone());
            let response = self
                .responses
                .lock()
                .unwrap()
                .get(&request.url)
                .cloned()
                .ok_or(TransportError::Unavailable)?;
            Ok(HttpResponse {
                timing: self
                    .timings
                    .lock()
                    .unwrap()
                    .get(&request.url)
                    .cloned()
                    .unwrap_or_default(),
                upload: *self.upload.lock().unwrap(),
                operation_id: request.operation_id,
                status: response.0,
                headers: response.1,
                body: Box::pin(stream::iter(vec![Ok(Bytes::from(response.2))])),
            })
        }

        async fn cancel(&self, operation_id: u64) -> Result<(), TransportError> {
            self.cancelled.lock().unwrap().push(operation_id);
            Ok(())
        }
    }

    struct DenyPrivate;

    impl FetchPolicy for DenyPrivate {
        fn validate_request(&self, request: &HttpRequest) -> Result<(), String> {
            (!request.url.contains("127.0.0.1"))
                .then_some(())
                .ok_or_else(|| "private network".into())
        }

        fn validate_response_headers(
            &self,
            _request: &HttpRequest,
            _status: u16,
            headers: &[(String, String)],
        ) -> Result<(), String> {
            (!headers.iter().any(|(name, _)| name == "x-deny"))
                .then_some(())
                .ok_or_else(|| "withheld by CORS/CSP".into())
        }
    }

    fn request(id: u64, url: &str) -> HttpRequest {
        HttpRequest {
            dns_timing: None,
            operation_id: id,
            method: "GET".into(),
            url: url.into(),
            headers: vec![],
            body: vec![],
            resolved_addrs: vec![],
        }
    }

    #[tokio::test]
    async fn response_preserves_final_transport_snapshot_without_inventing_body_completion() {
        for timing in [
            crate::TransportTiming::default(),
            crate::TransportTiming {
                measured_response: true,
                delivery: oryn_common::transport_timing::ResponseDelivery::Network,
                dns_start: Some(0),
                dns_end: Some(13),
                connect_start: Some(13),
                connect_end: Some(21),
                secure_start: Some(21),
                secure_end: Some(34),
                request_start: 34,
                response_start: 55,
                next_hop_protocol: "h2".into(),
                encoded_body_size: Some(7),
                ..Default::default()
            },
        ] {
            let transport = Arc::new(MockTransport::default());
            transport.responses.lock().unwrap().insert(
                "https://safe.test/body".into(),
                (200, vec![], b"decoded body".to_vec()),
            );
            transport
                .timings
                .lock()
                .unwrap()
                .insert("https://safe.test/body".into(), timing.clone());
            let pipeline =
                BrowserFetchPipeline::new(transport, Arc::new(DenyPrivate), FetchLimits::default());
            let mut response = pipeline
                .start(request(21, "https://safe.test/body"))
                .await
                .unwrap();
            assert_eq!(response.timing, timing);
            assert!(response.body.next().await.unwrap().is_ok());
            assert!(response.body.next().await.is_none());
            assert_eq!(
                response.timing, timing,
                "decoded body EOF does not fabricate transport measurements"
            );
        }
    }

    #[tokio::test]
    async fn redirected_fetch_replaces_dns_custody_and_returns_only_final_hop_timing() {
        let transport = Arc::new(MockTransport::default());
        transport.responses.lock().unwrap().extend([
            (
                "https://first.test/start".into(),
                (
                    302,
                    vec![("location".into(), "https://second.test/end".into())],
                    vec![],
                ),
            ),
            ("https://second.test/end".into(), (200, vec![], vec![])),
        ]);
        let first = crate::TransportTiming {
            measured_response: true,
            response_start: 900,
            next_hop_protocol: "http/1.1".into(),
            ..Default::default()
        };
        let last = crate::TransportTiming {
            measured_response: true,
            request_start: 17,
            response_start: 23,
            next_hop_protocol: "h2".into(),
            ..Default::default()
        };
        transport.timings.lock().unwrap().extend([
            ("https://first.test/start".into(), first),
            ("https://second.test/end".into(), last.clone()),
        ]);
        let pipeline = BrowserFetchPipeline::new(
            transport.clone(),
            Arc::new(DenyPrivate),
            FetchLimits::default(),
        );
        let mut input = request(22, "https://first.test/start");
        let start = std::time::Instant::now();
        input.dns_timing = Some(crate::DnsTiming {
            start,
            end: start + std::time::Duration::from_micros(5),
        });
        input.resolved_addrs.push("192.0.2.1:443".parse().unwrap());
        let response = pipeline.start(input).await.unwrap();
        assert_eq!(response.timing, last);
        assert_eq!(response.url, "https://second.test/end");
        assert!(response.redirected);
        let requests = transport.requests.lock().unwrap();
        assert!(requests[0].dns_timing.is_some());
        assert!(requests[1].dns_timing.is_none());
        assert!(requests[1].resolved_addrs.is_empty());
        assert_eq!(*transport.cancelled.lock().unwrap(), vec![22]);
    }

    #[tokio::test]
    async fn response_preserves_measured_upload_state_without_assuming_completion() {
        for upload in [
            None,
            Some(crate::UploadProgress {
                bytes_consumed: 2,
                total_bytes: 4,
                body_complete: false,
            }),
            Some(crate::UploadProgress {
                bytes_consumed: 4,
                total_bytes: 4,
                body_complete: true,
            }),
        ] {
            let transport = Arc::new(MockTransport::default());
            *transport.upload.lock().unwrap() = upload;
            transport
                .responses
                .lock()
                .unwrap()
                .insert("https://safe.test/upload".into(), (200, vec![], vec![]));
            let pipeline =
                BrowserFetchPipeline::new(transport, Arc::new(DenyPrivate), FetchLimits::default());
            let mut input = request(11, "https://safe.test/upload");
            input.method = "POST".into();
            input.body = b"body".to_vec();
            let response = pipeline.start(input).await.unwrap();
            assert_eq!(response.upload, upload);
        }
    }

    #[tokio::test]
    async fn default_redirect_limit_accepts_twenty_hops_and_rejects_twenty_one() {
        for hops in [20, 21] {
            let transport = Arc::new(MockTransport::default());
            for index in 0..=hops {
                let url = format!("https://safe.test/{index}");
                let headers = if index < hops {
                    vec![("location".into(), format!("/{next}", next = index + 1))]
                } else {
                    vec![]
                };
                transport
                    .responses
                    .lock()
                    .unwrap()
                    .insert(url, (if index < hops { 302 } else { 200 }, headers, vec![]));
            }
            let pipeline = BrowserFetchPipeline::new(
                transport.clone(),
                Arc::new(DenyPrivate),
                FetchLimits::default(),
            );
            let result = pipeline.start(request(10, "https://safe.test/0")).await;
            if hops == 20 {
                let response = result.unwrap();
                assert_eq!(response.status, 200);
                assert!(response.redirected);
            } else {
                assert!(matches!(result, Err(FetchError::TooManyRedirects)));
            }
            assert_eq!(transport.requests.lock().unwrap().len(), 21);
        }
    }

    #[tokio::test]
    async fn redirected_requests_preserve_head_and_remove_rewritten_body_headers() {
        for (status, method, expected) in [
            (303, "HEAD", "HEAD"),
            (303, "POST", "GET"),
            (302, "POST", "GET"),
            (307, "POST", "POST"),
            (308, "PUT", "PUT"),
        ] {
            let transport = Arc::new(MockTransport::default());
            transport.responses.lock().unwrap().extend([
                (
                    "https://first.test/start".into(),
                    (
                        status,
                        vec![("location".into(), "https://second.test/end".into())],
                        vec![],
                    ),
                ),
                ("https://second.test/end".into(), (200, vec![], vec![])),
            ]);
            let pipeline = BrowserFetchPipeline::new(
                transport.clone(),
                Arc::new(DenyPrivate),
                FetchLimits::default(),
            );
            let mut input = request(1, "https://first.test/start");
            input.method = method.into();
            input.body = b"body".to_vec();
            input.headers = vec![
                ("Content-Type".into(), "text/plain".into()),
                ("Content-Length".into(), "4".into()),
                ("Authorization".into(), "Bearer test".into()),
            ];
            input.resolved_addrs = vec!["192.0.2.1:443".parse().unwrap()];
            assert!(pipeline.start(input).await.unwrap().redirected);
            let seen = transport.requests.lock().unwrap();
            let next = &seen[1];
            assert_eq!(next.method, expected);
            assert!(next.resolved_addrs.is_empty());
            assert!(
                !next
                    .headers
                    .iter()
                    .any(|(name, _)| name.eq_ignore_ascii_case("authorization"))
            );
            if method != expected {
                assert!(next.body.is_empty());
                assert!(next.headers.is_empty());
            } else {
                assert_eq!(next.body, b"body");
                assert_eq!(next.headers.len(), 2);
            }
        }
    }

    #[tokio::test]
    async fn redirect_status_without_location_returns_the_response_body() {
        let transport = Arc::new(MockTransport::default());
        transport.responses.lock().unwrap().insert(
            "https://safe.test/start".into(),
            (302, vec![], b"ordinary response".to_vec()),
        );
        let pipeline = BrowserFetchPipeline::new(
            transport.clone(),
            Arc::new(DenyPrivate),
            FetchLimits::default(),
        );
        let mut response = pipeline
            .start(request(2, "https://safe.test/start"))
            .await
            .unwrap();
        assert_eq!(response.status, 302);
        assert!(!response.redirected);
        assert_eq!(
            response.body.next().await.unwrap().unwrap(),
            Bytes::from_static(b"ordinary response")
        );
        assert!(transport.cancelled.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn every_redirect_is_revalidated_before_the_next_request() {
        let transport = Arc::new(MockTransport::default());
        transport.responses.lock().unwrap().insert(
            "https://safe.test/start".into(),
            (
                302,
                vec![("location".into(), "http://127.0.0.1/secret".into())],
                vec![],
            ),
        );
        let pipeline = BrowserFetchPipeline::new(
            transport.clone(),
            Arc::new(DenyPrivate),
            FetchLimits::default(),
        );
        assert!(matches!(
            pipeline.start(request(7, "https://safe.test/start")).await,
            Err(FetchError::RequestDenied(_))
        ));
        assert_eq!(*transport.cancelled.lock().unwrap(), [7]);
    }

    #[tokio::test]
    async fn response_bytes_are_withheld_until_headers_pass_policy() {
        let transport = Arc::new(MockTransport::default());
        transport.responses.lock().unwrap().insert(
            "https://safe.test/data".into(),
            (200, vec![("x-deny".into(), "1".into())], b"secret".to_vec()),
        );
        let pipeline = BrowserFetchPipeline::new(
            transport.clone(),
            Arc::new(DenyPrivate),
            FetchLimits::default(),
        );
        assert!(matches!(
            pipeline.start(request(9, "https://safe.test/data")).await,
            Err(FetchError::ResponseDenied(_))
        ));
        assert_eq!(*transport.cancelled.lock().unwrap(), [9]);
    }

    #[tokio::test]
    async fn accepted_body_remains_incremental() {
        let transport = Arc::new(MockTransport::default());
        transport.responses.lock().unwrap().insert(
            "https://safe.test/data".into(),
            (200, vec![], b"streamed".to_vec()),
        );
        let pipeline =
            BrowserFetchPipeline::new(transport, Arc::new(DenyPrivate), FetchLimits::default());
        let response = pipeline
            .start(request(11, "https://safe.test/data"))
            .await
            .unwrap();
        let body = response
            .body
            .map(|chunk| chunk.unwrap())
            .collect::<Vec<_>>()
            .await;
        assert_eq!(body.concat(), b"streamed");
    }
}
