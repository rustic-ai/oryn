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
            max_redirects: 10,
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
        let original_url = request.url.clone();
        for redirect_count in 0..=self.limits.max_redirects {
            self.policy
                .validate_request(&request)
                .map_err(FetchError::RequestDenied)?;
            let HttpResponse {
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
            if is_redirect(status) {
                if redirect_count == self.limits.max_redirects {
                    let _ = self.transport.cancel(operation_id).await;
                    return Err(FetchError::TooManyRedirects);
                }
                let location = header(&headers, "location").ok_or(FetchError::InvalidRedirect)?;
                let base = Url::parse(&request.url).map_err(|_| FetchError::InvalidRedirect)?;
                let next = base
                    .join(location)
                    .map_err(|_| FetchError::InvalidRedirect)?;
                let _ = self.transport.cancel(operation_id).await;
                request.url = next.to_string();
                if status == 303 || matches!(status, 301 | 302) && request.method == "POST" {
                    request.method = "GET".into();
                    request.body.clear();
                }
                continue;
            }
            let encoding = ContentEncoding::parse(header(&headers, "content-encoding"))?;
            let body = decode_content_stream(
                body,
                encoding,
                self.limits.max_compressed_body_bytes,
                self.limits.max_decoded_body_bytes,
            );
            let redirected = request.url != original_url;
            return Ok(BrowserResponse {
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

const fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
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
        responses: Mutex<BTreeMap<String, MockResponse>>,
        cancelled: Mutex<Vec<u64>>,
    }

    #[async_trait]
    impl HttpTransport for MockTransport {
        async fn start(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
            let response = self
                .responses
                .lock()
                .unwrap()
                .get(&request.url)
                .cloned()
                .ok_or(TransportError::Unavailable)?;
            Ok(HttpResponse {
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
            operation_id: id,
            method: "GET".into(),
            url: url.into(),
            headers: vec![],
            body: vec![],
            resolved_addrs: vec![],
        }
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
