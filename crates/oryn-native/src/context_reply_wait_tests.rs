//! Deterministic channel ordering and real Unix transport receipt regressions.
use super::*;

fn response(value: Result<ContextServiceResponse, String>) -> ContextReply {
    Ok(ContextDelivery {
        decode_ns: 0,
        ready_at: Instant::now(),
        response: value,
    })
}
fn success() -> ContextReply {
    response(Ok(ContextServiceResponse::ReferrerPolicy {
        value: "ready".into(),
    }))
}

#[test]
fn context_reply_wait_does_not_consume_queued_response_before_publication() {
    let (reply, mut wait) = ContextReplyWait::new(None);
    let consumed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (entered, entering) = mpsc::channel();
    wait.before_wait = Some(entered);
    wait.response_consumed = Some(consumed.clone());
    assert!(reply.response.send(success()).is_ok());
    let (done, finished) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let result = wait.receive();
        let _ = done.send(result);
    });
    let entered = entering.recv_timeout(std::time::Duration::from_secs(5));
    let consumed_early = consumed.load(Ordering::SeqCst);
    let completed_early = finished.try_recv();
    let published = reply.publication.send(Ok(()));
    let result = finished.recv_timeout(std::time::Duration::from_secs(5));
    worker.join().unwrap();
    assert!(entered.is_ok());
    assert!(!consumed_early);
    assert!(matches!(completed_early, Err(mpsc::TryRecvError::Empty)));
    assert!(published.is_ok());
    let (delivery, publication) = result
        .unwrap_or_else(|_| panic!("completion missing"))
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(publication.is_ok());
    assert!(consumed.load(Ordering::SeqCst));
    assert!(
        matches!(delivery.response, Ok(ContextServiceResponse::ReferrerPolicy { value }) if value == "ready")
    );
}

#[test]
fn context_reply_wait_preserves_outer_error_when_publication_also_fails() {
    for error in [
        "context response protocol mismatch",
        "context service connection closed",
    ] {
        let (reply, wait) = ContextReplyWait::new(None);
        assert!(reply.response.send(Err(error.into())).is_ok());
        drop(reply);
        assert!(
            matches!(wait.receive(), Err(RuntimeError::ContextService(value)) if value == error)
        );
    }
}

#[test]
fn context_reply_wait_publication_failure_precedes_application_error() {
    let (reply, wait) = ContextReplyWait::new(None);
    assert!(
        reply
            .response
            .send(response(Err("application failure".into())))
            .is_ok()
    );
    assert!(
        reply
            .publication
            .send(Err("response receipt was not published".into()))
            .is_ok()
    );
    let (delivery, publication) = wait.receive().unwrap_or_else(|e| panic!("{e}"));
    assert!(
        matches!(publication, Err(RuntimeError::ContextService(value)) if value == "response receipt was not published")
    );
    assert!(matches!(delivery.response, Err(value) if value == "application failure"));
}

#[test]
fn context_reply_wait_abandoned_consumer_cannot_accept_response() {
    let (reply, wait) = ContextReplyWait::new(None);
    drop(wait);
    assert!(reply.response.send(success()).is_err());
    assert!(reply.publication.send(Ok(())).is_err());
}

fn read_frame<T: serde::de::DeserializeOwned>(reader: &mut std::io::BufReader<UnixStream>) -> T {
    let mut line = Vec::new();
    assert!(read_bounded_line(reader, &mut line, MAX_CONTEXT_FRAME_BYTES).unwrap() > 0);
    serde_json::from_slice(&line).unwrap()
}
fn request() -> ContextServiceRequest {
    ContextServiceRequest::GetReferrerPolicy {
        page_id: PageId(1),
        creator: oryn_js::worker_wire::Creator::Window {
            target: oryn_js::message_wire::WindowTarget::Root,
            document_generation: 1,
            stream_epoch: 0,
        },
    }
}

#[test]
fn context_reply_wait_unix_transport_preserves_receipt_before_next_request() {
    let (client, mut server) = UnixStream::pair().unwrap();
    let peer = std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(server.try_clone().unwrap());
        for index in 1..=2 {
            let request: RequestEnvelope = read_frame(&mut reader);
            assert_eq!(request.message_id, index);
            let envelope = ResponseEnvelope {
                protocol_version: CONTEXT_SERVICE_PROTOCOL_VERSION,
                route: request.route,
                correlation_id: request.message_id,
                response: (index == 2).then(|| ContextServiceResponse::ReferrerPolicy {
                    value: "ready".into(),
                }),
                error: (index == 1).then(|| "application failure".into()),
            };
            let (bytes, preserved) =
                encode_context_response_status(envelope, MAX_CONTEXT_FRAME_BYTES).unwrap();
            assert!(preserved);
            server.write_all(&bytes).unwrap();
            let receipt: ResponseReceipt = read_frame(&mut reader);
            assert_eq!(receipt.received_id, index);
            assert!(receipt.accepted);
        }
    });
    let transport = IpcContextTransport::new(IpcContextClient::new(client)).unwrap();
    let first = transport.request(request());
    let second = transport.request(request());
    drop(transport);
    peer.join().unwrap();
    assert!(
        matches!(first, Err(RuntimeError::ContextService(value)) if value == "application failure")
    );
    assert!(
        matches!(second, Ok(ContextServiceResponse::ReferrerPolicy { value }) if value == "ready")
    );
}

#[test]
fn context_reply_wait_unix_transport_ack_failure_wakes_and_joins_reader() {
    let (client, mut server) = UnixStream::pair().unwrap();
    let (observed, observation) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let peer = std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(server.try_clone().unwrap());
        let request: RequestEnvelope = read_frame(&mut reader);
        observed.send(()).unwrap();
        released.recv().unwrap();
        let envelope = ResponseEnvelope {
            protocol_version: CONTEXT_SERVICE_PROTOCOL_VERSION,
            route: request.route,
            correlation_id: request.message_id,
            response: Some(ContextServiceResponse::ReferrerPolicy {
                value: "must not escape".into(),
            }),
            error: None,
        };
        let (bytes, _) = encode_context_response_status(envelope, MAX_CONTEXT_FRAME_BYTES).unwrap();
        server.write_all(&bytes).unwrap();
    });
    let transport = Arc::new(IpcContextTransport::new(IpcContextClient::new(client)).unwrap());
    let caller = {
        let transport = transport.clone();
        std::thread::spawn(move || transport.request(request()))
    };
    observation.recv().unwrap();
    // Force the actual receipt writer's failure after request admission.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = transport.writer.lock().unwrap();
        panic!("injected receipt writer poison");
    }));
    release.send(()).unwrap();
    let result = caller.join().unwrap();
    peer.join().unwrap();
    drop(transport); // Joins the native response reader.
    assert!(
        matches!(result, Err(RuntimeError::ContextService(value)) if value == "response receipt was not published")
    );
}

#[test]
fn context_reply_wait_protocol_error_does_not_wait_for_rejected_ack_writer() {
    let (client, mut server) = UnixStream::pair().unwrap();
    let (observed, observation) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let peer = std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(server.try_clone().unwrap());
        let request: RequestEnvelope = read_frame(&mut reader);
        observed.send(()).unwrap();
        released.recv().unwrap();
        let envelope = ResponseEnvelope {
            protocol_version: CONTEXT_SERVICE_PROTOCOL_VERSION + 1,
            route: request.route,
            correlation_id: request.message_id,
            response: Some(ContextServiceResponse::ReferrerPolicy {
                value: "invalid".into(),
            }),
            error: None,
        };
        let (bytes, _) = encode_context_response_status(envelope, MAX_CONTEXT_FRAME_BYTES).unwrap();
        server.write_all(&bytes).unwrap();
        let receipt: ResponseReceipt = read_frame(&mut reader);
        assert!(!receipt.accepted);
    });
    let transport = Arc::new(IpcContextTransport::new(IpcContextClient::new(client)).unwrap());
    let (finished, completion) = mpsc::channel();
    let caller = {
        let transport = transport.clone();
        std::thread::spawn(move || {
            let _ = finished.send(transport.request(request()));
        })
    };
    observation.recv().unwrap();
    let guard = transport.writer.lock().unwrap();
    release.send(()).unwrap();
    let result = completion.recv_timeout(std::time::Duration::from_secs(5));
    drop(guard); // Always unblock native cleanup before making assertions.
    caller.join().unwrap();
    peer.join().unwrap();
    drop(transport);
    assert!(
        matches!(result, Ok(Err(RuntimeError::ContextService(value))) if value == "context response protocol mismatch")
    );
}

#[test]
fn context_reply_wait_unix_cancellation_drains_pending_and_joins_reader() {
    let (client, server) = UnixStream::pair().unwrap();
    let (observed, observation) = mpsc::channel();
    let peer = std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(server);
        let request: RequestEnvelope = read_frame(&mut reader);
        assert_eq!(request.message_id, 1);
        observed.send(()).unwrap();
        let mut line = Vec::new();
        assert_eq!(
            read_bounded_line(&mut reader, &mut line, MAX_CONTEXT_FRAME_BYTES).unwrap(),
            0
        );
    });
    let transport = Arc::new(IpcContextTransport::new(IpcContextClient::new(client)).unwrap());
    let caller = {
        let transport = transport.clone();
        std::thread::spawn(move || transport.request(request()))
    };
    observation.recv().unwrap();
    transport.cancel.shutdown(Shutdown::Both).unwrap();
    let result = caller.join().unwrap();
    peer.join().unwrap();
    drop(transport);
    assert!(
        matches!(result, Err(RuntimeError::ContextService(value)) if value == "context service connection closed")
    );
}
