//! A frame-principal-bound paint image request prepared on the actor and run off it.
use super::*;

pub(crate) struct SelectedPaintImageLoad {
    #[cfg(unix)]
    client: crate::context_service::SharedContextClient,
    #[cfg(unix)]
    page_id: oryn_common::v3::PageId,
    #[cfg(unix)]
    creator: oryn_js::worker_wire::Creator,
    #[cfg(unix)]
    request_id: u64,
    url: String,
    limit: usize,
}
impl SelectedPaintImageLoad {
    #[cfg(unix)]
    pub(super) fn new(
        client: crate::context_service::SharedContextClient,
        page_id: oryn_common::v3::PageId,
        creator: oryn_js::worker_wire::Creator,
        request_id: u64,
        url: String,
        limit: usize,
    ) -> Self {
        Self {
            client,
            page_id,
            creator,
            request_id,
            url,
            limit,
        }
    }
    /// A capture can only narrow the actor's original resource allowance.
    pub(crate) fn with_limit(mut self, limit: usize) -> Self {
        self.limit = self.limit.min(limit);
        self
    }
    /// Cancellation/deadline checks surround each synchronous context RPC. The
    /// worker retains this request until any in-flight RPC and cleanup return.
    pub(crate) fn run(
        self,
        cancelled: &std::sync::atomic::AtomicBool,
        deadline: std::time::Instant,
    ) -> Result<oryn_common::v4::PaintImageResource, HostError> {
        self.run_checked(|| {
            if cancelled.load(std::sync::atomic::Ordering::Acquire) {
                return Err(HostError::Execution(
                    "native paint request cancelled".into(),
                ));
            }
            if std::time::Instant::now() >= deadline {
                return Err(HostError::Execution(
                    "native paint request deadline exceeded".into(),
                ));
            }
            Ok(())
        })
    }
    pub(super) fn run_checked(
        self,
        mut check: impl FnMut() -> Result<(), HostError>,
    ) -> Result<oryn_common::v4::PaintImageResource, HostError> {
        check()?;
        if self.url.len() > 64 * 1024 || self.limit == 0 || self.limit > 64 * 1024 * 1024 {
            return Err(HostError::Execution(
                "frame image request exceeds bounds".into(),
            ));
        }
        #[cfg(not(unix))]
        {
            return Err(HostError::Execution(
                "frame resource authority bridge unavailable".into(),
            ));
        }
        #[cfg(unix)]
        {
            use crate::context_service::{
                ContextServiceRequest as Q, ContextServiceResponse as R, HttpStreamRequest,
            };
            let Self {
                client,
                page_id,
                creator,
                request_id,
                url,
                limit,
            } = self;
            let url = url.as_str();
            let mut request_cleanup = RequestCleanup {
                client: client.clone(),
                page_id,
                request_id,
                armed: true,
            };
            let result = (|| {
                let response = client
                    .request(Q::ImageStreamStart {
                        page_id,
                        cors_enabled: false,
                        request: HttpStreamRequest {
                            upgrade_mixed_image: false,
                            fetch: None,
                            client: Some(creator.clone()),
                            use_cors_preflight: false,
                            request_id,
                            method: "GET".into(),
                            url: url.into(),
                            headers: Vec::new(),
                            body_base64: String::new(),
                            credentials: true,
                            credentials_mode: Some("include".into()),
                            initiator_url: None,
                            referrer_policy: None,
                        },
                    })
                    .map_err(|error| HostError::Execution(error.to_string()))?;
                let R::HttpStreamStarted {
                    request_id: actual_request_id,
                    stream_id,
                    url: final_url,
                    status,
                    headers,
                    total_bytes,
                    ..
                } = response
                else {
                    return Err(HostError::Execution(
                        "invalid frame image stream response".into(),
                    ));
                };
                if actual_request_id != request_id {
                    return Err(HostError::Execution(
                        "frame image request identity mismatch".into(),
                    ));
                }
                request_cleanup.armed = false;
                let mut stream_cleanup = StreamCleanup {
                    client: client.clone(),
                    page_id,
                    creator: creator.clone(),
                    stream_id,
                    armed: true,
                };
                let result = (|| {
                    check()?;
                    if !(200..300).contains(&status)
                        || total_bytes.is_some_and(|bytes| bytes > limit)
                        || final_url.len() > 64 * 1024
                    {
                        return Err(HostError::Execution(
                            "frame image response rejected by size/status bounds".into(),
                        ));
                    }
                    let media_type = oryn_common::mime_type::MimeType::from_headers(&headers)
                        .map(|mime| mime.essence().to_owned())
                        .unwrap_or_else(|| "application/octet-stream".into());
                    let mut bytes = Vec::new();
                    for _ in 0..1024 {
                        check()?;
                        let chunk = client
                            .request(Q::HttpStreamRead {
                                page_id,
                                creator: Some(creator.clone()),
                                stream_id,
                                max_bytes: 64 * 1024,
                            })
                            .map_err(|error| HostError::Execution(error.to_string()))?;
                        check()?;
                        let R::HttpStreamChunk {
                            stream_id: actual,
                            body_base64,
                            done,
                            encoded_body_size: _,
                        } = chunk
                        else {
                            return Err(HostError::Execution("invalid frame image chunk".into()));
                        };
                        if actual != stream_id || body_base64.len() > 96 * 1024 {
                            return Err(HostError::Execution(
                                "frame image chunk identity/size mismatch".into(),
                            ));
                        }
                        let chunk = base64::Engine::decode(
                            &base64::engine::general_purpose::STANDARD,
                            body_base64,
                        )
                        .map_err(|error| HostError::Execution(error.to_string()))?;
                        if bytes.len().saturating_add(chunk.len()) > limit {
                            return Err(HostError::Execution(
                                "frame image body exceeds bounds".into(),
                            ));
                        }
                        bytes.extend(chunk);
                        if done {
                            check()?;
                            return Ok(oryn_common::v4::PaintImageResource {
                                svg_document_snapshot: false,
                                svg_instances: Vec::new(),
                                content_type: oryn_common::xml_encoding::response_content_type(
                                    &headers,
                                ),
                                url_aliases: if final_url != url {
                                    vec![url.into()]
                                } else {
                                    Vec::new()
                                },
                                url: final_url,
                                media_type,
                                bytes,
                            });
                        }
                    }
                    Err(HostError::Execution(
                        "frame image chunk count exceeds bounds".into(),
                    ))
                })();
                merge_cleanup(result, stream_cleanup.finish()).and_then(|resource| {
                    check()?;
                    Ok(resource)
                })
            })();
            merge_cleanup(result, request_cleanup.finish())
        }
    }
}
#[cfg(unix)]
struct RequestCleanup {
    client: crate::context_service::SharedContextClient,
    page_id: oryn_common::v3::PageId,
    request_id: u64,
    armed: bool,
}
#[cfg(unix)]
fn merge_cleanup<T>(
    primary: Result<T, HostError>,
    cleanup: Result<(), HostError>,
) -> Result<T, HostError> {
    match (primary, cleanup) {
        (result, Ok(())) => result,
        (Ok(_), Err(cleanup)) => Err(cleanup),
        (Err(primary), Err(cleanup)) => Err(HostError::Execution(format!(
            "{primary}; secondary cleanup failure: {cleanup}"
        ))),
    }
}
#[cfg(unix)]
impl RequestCleanup {
    fn finish(&mut self) -> Result<(), HostError> {
        if !std::mem::replace(&mut self.armed, false) {
            return Ok(());
        }
        match self.client.request(
            crate::context_service::ContextServiceRequest::HttpRequestAbort {
                page_id: self.page_id,
                request_id: self.request_id,
            },
        ) {
            Ok(crate::context_service::ContextServiceResponse::HttpRequestAborted) => Ok(()),
            Ok(_) => Err(HostError::Execution(
                "frame image request cleanup returned wrong acknowledgement".into(),
            )),
            Err(error) => Err(HostError::Execution(format!(
                "frame image request cleanup failed: {error}"
            ))),
        }
    }
}
#[cfg(unix)]
impl Drop for RequestCleanup {
    fn drop(&mut self) {
        if let Err(error) = self.finish() {
            eprintln!("native paint fallback cleanup: {error}");
        }
    }
}
#[cfg(unix)]
struct StreamCleanup {
    client: crate::context_service::SharedContextClient,
    page_id: oryn_common::v3::PageId,
    creator: oryn_js::worker_wire::Creator,
    stream_id: u64,
    armed: bool,
}
#[cfg(unix)]
impl StreamCleanup {
    fn finish(&mut self) -> Result<(), HostError> {
        if !std::mem::replace(&mut self.armed, false) {
            return Ok(());
        }
        match self.client.request(
            crate::context_service::ContextServiceRequest::HttpStreamAbort {
                page_id: self.page_id,
                creator: Some(self.creator.clone()),
                stream_id: self.stream_id,
            },
        ) {
            Ok(crate::context_service::ContextServiceResponse::HttpStreamAborted) => Ok(()),
            Ok(_) => Err(HostError::Execution(
                "frame image stream cleanup returned wrong acknowledgement".into(),
            )),
            Err(error) => Err(HostError::Execution(format!(
                "frame image stream cleanup failed: {error}"
            ))),
        }
    }
}
#[cfg(unix)]
impl Drop for StreamCleanup {
    fn drop(&mut self) {
        if let Err(error) = self.finish() {
            eprintln!("native paint fallback cleanup: {error}");
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::context_service::{
        ContextServiceRequest as Q, ContextServiceResponse as R, ContextTransport,
        SharedContextClient,
    };
    use std::{
        collections::VecDeque,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        },
        time::{Duration, Instant},
    };

    struct Transport {
        replies: Mutex<VecDeque<Result<R, crate::RuntimeError>>>,
        calls: Mutex<Vec<Q>>,
        abort_reply: Mutex<Option<Result<R, crate::RuntimeError>>>,
        cancelled: Arc<AtomicBool>,
        cancel_on_read: bool,
    }
    impl ContextTransport for Transport {
        fn request_routed(
            &self,
            route: Option<&crate::worker_route::WorkerRoute>,
            request: Q,
        ) -> Result<R, crate::RuntimeError> {
            assert!(route.is_some());
            self.request(request)
        }
        fn request(&self, request: Q) -> Result<R, crate::RuntimeError> {
            self.calls.lock().unwrap().push(request.clone());
            match request {
                Q::HttpStreamAbort { .. } => self
                    .abort_reply
                    .lock()
                    .unwrap()
                    .take()
                    .unwrap_or(Ok(R::HttpStreamAborted)),
                Q::HttpRequestAbort { .. } => self
                    .abort_reply
                    .lock()
                    .unwrap()
                    .take()
                    .unwrap_or(Ok(R::HttpRequestAborted)),
                _ => {
                    if self.cancel_on_read && matches!(request, Q::HttpStreamRead { .. }) {
                        self.cancelled.store(true, Ordering::Release);
                    }
                    self.replies
                        .lock()
                        .unwrap()
                        .pop_front()
                        .expect("unexpected request")
                }
            }
        }
    }
    fn started(status: u16, total: Option<usize>) -> R {
        R::HttpStreamStarted {
            timing: None,
            internal_response: None,
            response_type: None,
            upload: None,
            cors_filtered: false,
            url: "https://frame.test/final.png".into(),
            redirected: true,
            stream_id: 41,
            request_id: 99,
            status,
            headers: vec![("content-type".into(), "image/png".into())],
            total_bytes: total,
        }
    }
    fn chunk(stream_id: u64, body: &str) -> R {
        R::HttpStreamChunk {
            encoded_body_size: None,
            stream_id,
            body_base64: body.into(),
            done: true,
        }
    }
    fn load(
        replies: Vec<Result<R, crate::RuntimeError>>,
        cancel_on_read: bool,
    ) -> (SelectedPaintImageLoad, Arc<Transport>) {
        let transport = Arc::new(Transport {
            replies: Mutex::new(replies.into()),
            calls: Mutex::new(Vec::new()),
            abort_reply: Mutex::new(None),
            cancelled: Arc::new(AtomicBool::new(false)),
            cancel_on_read,
        });
        let client = SharedContextClient::for_test_transport(
            transport.clone(),
            crate::worker_route::WorkerRoute::issue(811, 5, 0).unwrap(),
        );
        let creator = oryn_js::worker_wire::Creator::Window {
            target: oryn_js::message_wire::WindowTarget::Frame("captured-frame".into()),
            document_generation: 7,
            stream_epoch: 3,
        };
        (
            SelectedPaintImageLoad::new(
                client,
                oryn_common::v3::PageId(5),
                creator,
                99,
                "https://frame.test/original.png".into(),
                32,
            ),
            transport,
        )
    }
    #[test]
    fn stream_cleanup_rejects_failed_or_wrong_ack_after_success_and_failure() {
        for failed_read in [false, true] {
            for wrong_ack in [false, true] {
                let read = if failed_read {
                    Err(crate::RuntimeError::Network("primary read failure".into()))
                } else {
                    Ok(chunk(41, "aGk="))
                };
                let (load, transport) = load(vec![Ok(started(200, None)), read], false);
                *transport.abort_reply.lock().unwrap() = Some(if wrong_ack {
                    Ok(R::HttpRequestAborted)
                } else {
                    Err(crate::RuntimeError::Network(
                        "abort transport failure".into(),
                    ))
                });
                let error = load.run_checked(|| Ok(())).unwrap_err().to_string();
                assert!(error.contains(if wrong_ack {
                    "wrong acknowledgement"
                } else {
                    "abort transport failure"
                }));
                if failed_read {
                    assert!(error.contains("primary read failure"));
                }
                assert_stream_closed(&transport);
            }
        }
    }
    #[test]
    fn start_failure_retains_primary_and_failed_request_abort() {
        for wrong_ack in [false, true] {
            let (load, transport) = load(
                vec![Err(crate::RuntimeError::Network("start failed".into()))],
                false,
            );
            *transport.abort_reply.lock().unwrap() = Some(if wrong_ack {
                Ok(R::HttpStreamAborted)
            } else {
                Err(crate::RuntimeError::Network("abort failed".into()))
            });
            let error = load.run_checked(|| Ok(())).unwrap_err().to_string();
            assert!(error.contains("start failed"));
            assert!(error.contains(if wrong_ack {
                "wrong acknowledgement"
            } else {
                "abort failed"
            }));
            assert_eq!(
                transport
                    .calls
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|q| matches!(q, Q::HttpRequestAbort { .. }))
                    .count(),
                1
            );
        }
    }
    fn assert_stream_closed(transport: &Transport) {
        let calls = transport.calls.lock().unwrap();
        assert_eq!(
            calls
                .iter()
                .filter(|q| matches!(q, Q::HttpStreamAbort { stream_id: 41, .. }))
                .count(),
            1
        );
        let Q::ImageStreamStart {
            page_id, request, ..
        } = &calls[0]
        else {
            panic!("image start")
        };
        assert_eq!(page_id.0, 5);
        assert_eq!(request.request_id, 99);
        for call in calls.iter().skip(1) {
            match call {
                Q::HttpStreamRead {
                    page_id: page,
                    creator,
                    ..
                }
                | Q::HttpStreamAbort {
                    page_id: page,
                    creator,
                    ..
                } => {
                    assert_eq!(page, page_id);
                    assert_eq!(creator, &request.client);
                }
                _ => panic!("unexpected protocol operation"),
            }
        }
    }
    #[test]
    fn sendable_descriptor_preserves_principal_redirect_alias_and_narrows_budget() {
        fn send<T: Send>() {}
        send::<SelectedPaintImageLoad>();
        let (request, transport) = load(
            vec![Ok(started(200, Some(3))), Ok(chunk(41, "YWJj"))],
            false,
        );
        let image = request
            .with_limit(3)
            .with_limit(64)
            .run(
                &transport.cancelled,
                Instant::now() + Duration::from_secs(30),
            )
            .unwrap();
        assert_eq!(image.bytes, b"abc");
        assert_eq!(image.media_type, "image/png");
        assert_eq!(image.url, "https://frame.test/final.png");
        assert_eq!(image.url_aliases, vec!["https://frame.test/original.png"]);
        assert_stream_closed(&transport);
        let (request, transport) = load(vec![Ok(started(200, None)), Ok(chunk(41, "YWJj"))], false);
        assert!(
            request
                .with_limit(2)
                .with_limit(64)
                .run(
                    &transport.cancelled,
                    Instant::now() + Duration::from_secs(30)
                )
                .is_err(),
            "later allowance cannot widen a previously narrowed body cap"
        );
        assert_stream_closed(&transport);
    }
    #[test]
    fn cancelled_or_expired_requests_never_start_io() {
        for cancelled in [false, true] {
            let (request, transport) = load(Vec::new(), false);
            transport.cancelled.store(cancelled, Ordering::Release);
            let deadline = if cancelled {
                Instant::now() + Duration::from_secs(30)
            } else {
                Instant::now()
            };
            assert!(request.run(&transport.cancelled, deadline).is_err());
            assert!(transport.calls.lock().unwrap().is_empty());
        }
    }
    #[test]
    fn cancellation_after_read_closes_owned_stream_before_return() {
        let (request, transport) = load(vec![Ok(started(200, None)), Ok(chunk(41, "YWJj"))], true);
        assert!(
            request
                .run(
                    &transport.cancelled,
                    Instant::now() + Duration::from_secs(30)
                )
                .is_err()
        );
        assert_stream_closed(&transport);
    }
    #[test]
    fn errors_and_deadline_checks_retire_streams_and_unresolved_start_requests() {
        for replies in [
            vec![Ok(started(500, None))],
            vec![Ok(started(200, Some(33)))],
            vec![Ok(started(200, None)), Ok(chunk(42, "YWJj"))],
            vec![Ok(started(200, None)), Ok(chunk(41, "!bad-base64!"))],
            vec![
                Ok(started(200, None)),
                Err(crate::RuntimeError::Network("read failed".into())),
            ],
        ] {
            let (request, transport) = load(replies, false);
            assert!(
                request
                    .run(
                        &transport.cancelled,
                        Instant::now() + Duration::from_secs(30)
                    )
                    .is_err()
            );
            assert_stream_closed(&transport);
        }
        // Deterministic deadline boundary after a stream is acquired, without
        // timing-sensitive sleeps. Public deadline rejection is covered above.
        let (request, transport) = load(vec![Ok(started(200, None))], false);
        let mut checks = 0;
        assert!(
            request
                .run_checked(|| {
                    checks += 1;
                    if checks == 2 {
                        Err(HostError::Execution(
                            "native paint request deadline exceeded".into(),
                        ))
                    } else {
                        Ok(())
                    }
                })
                .is_err()
        );
        assert_stream_closed(&transport);
        let (request, transport) = load(
            vec![Err(crate::RuntimeError::Network("start failed".into()))],
            false,
        );
        assert!(
            request
                .run(
                    &transport.cancelled,
                    Instant::now() + Duration::from_secs(30)
                )
                .is_err()
        );
        let calls = transport.calls.lock().unwrap();
        assert!(matches!(
            calls.as_slice(),
            [
                Q::ImageStreamStart { .. },
                Q::HttpRequestAbort { request_id: 99, .. }
            ]
        ));
    }
    #[test]
    fn blocked_read_and_abort_keep_worker_custody_until_both_return() {
        use std::sync::mpsc;
        struct Gated {
            inner: Arc<Transport>,
            entered: mpsc::Sender<Q>,
            release_read: Mutex<mpsc::Receiver<()>>,
            release_abort: Mutex<mpsc::Receiver<()>>,
        }
        impl ContextTransport for Gated {
            fn request_routed(
                &self,
                route: Option<&crate::worker_route::WorkerRoute>,
                request: Q,
            ) -> Result<R, crate::RuntimeError> {
                assert!(route.is_some());
                self.request(request)
            }
            fn request(&self, request: Q) -> Result<R, crate::RuntimeError> {
                let gate = match &request {
                    Q::HttpStreamRead { .. } => Some(&self.release_read),
                    Q::HttpStreamAbort { .. } => Some(&self.release_abort),
                    _ => None,
                };
                if let Some(gate) = gate {
                    self.entered.send(request.clone()).unwrap();
                    gate.lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(5))
                        .expect("release owned RPC gate");
                }
                self.inner.request(request)
            }
        }
        let (mut request, transport) =
            load(vec![Ok(started(200, None)), Ok(chunk(41, "YWJj"))], false);
        let (entered_tx, entered_rx) = mpsc::channel();
        let (read_tx, read_rx) = mpsc::channel();
        let (abort_tx, abort_rx) = mpsc::channel();
        let gated = Arc::new(Gated {
            inner: transport.clone(),
            entered: entered_tx,
            release_read: Mutex::new(read_rx),
            release_abort: Mutex::new(abort_rx),
        });
        request.client = SharedContextClient::for_test_transport(
            gated,
            crate::worker_route::WorkerRoute::issue(811, 5, 0).unwrap(),
        );
        let (result_tx, result_rx) = mpsc::channel();
        let cancelled = transport.cancelled.clone();
        std::thread::scope(|scope| {
            let worker = scope.spawn(move || {
                let result = request.run(&cancelled, Instant::now() + Duration::from_secs(30));
                result_tx
                    .send(result.map(|_| ()).map_err(|error| error.to_string()))
                    .unwrap();
            });
            let read = entered_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("worker entered read");
            let Q::HttpStreamRead {
                page_id,
                creator,
                stream_id: 41,
                ..
            } = read
            else {
                panic!("expected owned read")
            };
            assert_eq!(page_id.0, 5);
            transport.cancelled.store(true, Ordering::Release);
            assert!(
                matches!(result_rx.try_recv(), Err(mpsc::TryRecvError::Empty)),
                "cancellation cannot report completion while read RPC owns work"
            );
            assert!(!worker.is_finished());
            read_tx.send(()).unwrap();
            let abort = entered_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("worker entered stream cleanup");
            let Q::HttpStreamAbort {
                page_id: abort_page,
                creator: abort_creator,
                stream_id: 41,
            } = abort
            else {
                panic!("expected owned stream abort")
            };
            assert_eq!(abort_page, page_id);
            assert_eq!(abort_creator, creator);
            assert!(
                matches!(result_rx.try_recv(), Err(mpsc::TryRecvError::Empty)),
                "worker cannot publish completion while abort RPC is still held"
            );
            assert!(!worker.is_finished());
            abort_tx.send(()).unwrap();
            let error = result_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("completion after cleanup")
                .unwrap_err();
            assert!(error.contains("native paint request cancelled"));
            worker.join().unwrap();
        });
        assert_stream_closed(&transport);
    }

    #[test]
    fn malformed_and_foreign_start_responses_abort_only_the_owned_request() {
        let mut foreign = started(200, None);
        if let R::HttpStreamStarted {
            request_id,
            stream_id,
            ..
        } = &mut foreign
        {
            *request_id = 100;
            *stream_id = 777;
        }
        for response in [foreign, chunk(777, "YWJj")] {
            let (request, transport) = load(vec![Ok(response)], false);
            assert!(
                request
                    .run(
                        &transport.cancelled,
                        Instant::now() + Duration::from_secs(30)
                    )
                    .is_err()
            );
            let calls = transport.calls.lock().unwrap();
            assert!(
                matches!(
                    calls.as_slice(),
                    [
                        Q::ImageStreamStart {
                            page_id: oryn_common::v3::PageId(5),
                            ..
                        },
                        Q::HttpRequestAbort {
                            page_id: oryn_common::v3::PageId(5),
                            request_id: 99
                        }
                    ]
                ),
                "untrusted stream identity must not authorize stream cleanup"
            );
            assert!(
                !calls
                    .iter()
                    .any(|request| matches!(request, Q::HttpStreamAbort { .. }))
            );
        }
    }
}
