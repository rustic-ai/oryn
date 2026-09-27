//! Error custody while a committed navigation waits for its final acknowledgement.
use super::*;
use crate::navigation_control::{
    NavigationCommand, NavigationControl, NavigationEmitter, NavigationEvent, NavigationRequest,
    NavigationStop, PageLoadStrategy, PendingNavigationAck,
};
use crate::network::{NetworkError, NetworkEvent, NetworkOperation, NetworkResponse};

fn page(broker: Option<SharedNetworkBroker>) -> PageState {
    let mut page = PageState::new(
        PageId(941_089),
        None,
        BrowserConfig::default(),
        ContextOptions::default(),
        broker,
        #[cfg(unix)]
        None,
    );
    page.load_document(
        "http://runner.test/source".into(),
        "<title>Source</title>",
        None,
        None,
        HistoryUpdate::Push,
        &BTreeMap::new(),
        Vec::new(),
    )
    .unwrap();
    page
}

fn emitter() -> NavigationEmitter {
    let control = NavigationControl::default();
    let job = control
        .begin(NavigationRequest {
            command: NavigationCommand::Navigate {
                url: "https://runner.test/target".into(),
            },
            strategy: PageLoadStrategy::Normal,
            timeout_ms: None,
        })
        .unwrap();
    job.publish(NavigationEvent::Started).unwrap();
    NavigationEmitter::Local(job)
}

fn delayed_reply(
    page: &mut PageState,
    error: Option<RuntimeError>,
) -> oneshot::Receiver<Result<NavigationResult, RuntimeError>> {
    let executable = page.executable.as_ref().unwrap();
    let result = error.map_or_else(
        || {
            Ok(NavigationResult {
                page_id: page.page_id,
                requested_url: page.url.clone(),
                final_url: page.url.clone(),
                title: String::new(),
                status: 200,
                document_generation: page.document_generation,
                revision: Revision(0),
                lifecycle: page.lifecycle.clone(),
                challenge: page.last_challenge.clone(),
            })
        },
        Err,
    );
    let (sender, receiver) = oneshot::channel();
    page.settling_navigation = Some(NavigationSettlement {
        result,
        reply: Some(NavigationReply::Navigation(sender)),
        source_generation: page.document_generation.saturating_sub(1),
        deadline: Instant::now() + Duration::from_secs(2),
        identity: executable.form_history_root_identity(),
        load_epoch: executable.native_navigation_load_epoch(),
        ack: Some(PendingNavigationAck::test_pending()),
    });
    receiver
}

#[test]
fn task_failure_survives_delayed_ack_and_native_load_cleanup() {
    let mut page = page(None);
    let epoch = page
        .executable
        .as_ref()
        .unwrap()
        .native_navigation_load_epoch();
    page.active_navigation = Some(emitter());
    let mut reply = delayed_reply(&mut page, None);
    let cause = RuntimeError::Script("navigation body failed: explicit terminal failure".into());
    assert_eq!(page.stop_failed_navigation_load(cause.clone()), cause);
    assert_eq!(reply.try_recv().unwrap().unwrap_err(), cause);
    assert!(page.active_navigation.is_none());
    assert!(page.settling_navigation.is_none());
    assert_ne!(
        page.executable
            .as_ref()
            .unwrap()
            .native_navigation_load_epoch(),
        epoch
    );
    assert!(matches!(
        reply.try_recv(),
        Err(oneshot::error::TryRecvError::Closed)
    ));
}

#[test]
fn existing_settlement_failure_precedes_later_cancel_or_close() {
    for later in [RuntimeError::NavigationCancelled, RuntimeError::Closed] {
        let mut page = page(None);
        let first = RuntimeError::CertificateRejected("first response certificate".into());
        let mut reply = delayed_reply(&mut page, Some(first.clone()));
        page.cancel_navigation_transaction(later);
        assert_eq!(reply.try_recv().unwrap().unwrap_err(), first);
        assert!(page.settling_navigation.is_none());
    }
}

#[test]
fn checkpoint_rejection_preserves_typed_stop_and_cleans_pending_reply() {
    for reason in [
        NavigationStop::Timeout,
        NavigationStop::Retired,
        NavigationStop::CertificateRejected("checkpoint certificate".into()),
    ] {
        for already_stopped in [false, true] {
            let mut page = page(None);
            let emitter = emitter();
            page.active_navigation = Some(emitter.clone());
            let mut reply = delayed_reply(&mut page, None);
            if already_stopped {
                emitter
                    .publish(NavigationEvent::Stopped(reason.clone()))
                    .unwrap();
                // First poll obtains the local checkpoint acknowledgement.
                page.poll_navigation_control_checkpoint().unwrap();
            } else {
                let ack = PendingNavigationAck::test_pending();
                page.navigation_control_checkpoint = Some((emitter, ack.clone()));
                page.poll_navigation_control_checkpoint().unwrap();
                assert!(matches!(
                    reply.try_recv(),
                    Err(oneshot::error::TryRecvError::Empty)
                ));
                ack.test_finish(Err(reason.clone()));
            }
            let error = page.poll_navigation_control_checkpoint().unwrap_err();
            assert_eq!(error.navigation_stop(), reason);
            assert_eq!(
                reply.try_recv().unwrap().unwrap_err().navigation_stop(),
                reason
            );
            assert!(page.active_navigation.is_none());
            assert!(page.settling_navigation.is_none());
        }
    }
}

#[test]
fn cleanup_failure_round_trips_primary_kind_and_secondary_detail() {
    for cause in [
        RuntimeError::CertificateRejected("original certificate".into()),
        RuntimeError::NavigationStopped(NavigationStop::Timeout),
        RuntimeError::NavigationStopped(NavigationStop::Retired),
    ] {
        let mut page = page(None);
        let wrapped = page.finish_navigation_load_cleanup(
            cause.clone(),
            Err(RuntimeError::Script("secondary cleanup failure".into())),
        );
        assert!(page.trace.events().iter().any(|event| matches!(
            &event.kind, TraceEventKind::ScriptDiagnostic { capability, detail }
                if capability == "navigation.cleanup" && detail.contains("secondary cleanup failure")
        )));
        let encoded = serde_json::to_vec(&wrapped).unwrap();
        let decoded: RuntimeError = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, wrapped);
        assert_eq!(decoded.root_cause(), &cause);
        assert_eq!(decoded.navigation_stop(), cause.navigation_stop());
        assert!(decoded.to_string().contains("secondary cleanup failure"));
    }
    let mut page = page(None);
    let error = page.finish_navigation_load_cleanup(
        RuntimeError::NavigationStopped(NavigationStop::Timeout),
        Err(RuntimeError::Script("é".repeat(5000))),
    );
    assert_eq!(error.navigation_stop(), NavigationStop::Timeout);
    assert!(page.trace.events().iter().any(|event| matches!(
        &event.kind, TraceEventKind::ScriptDiagnostic { capability, detail }
            if capability == "navigation.cleanup" && detail.len() <= 4108 && detail.ends_with(" [truncated]")
    )));
}

struct RunnerBroker;
impl NetworkBroker for RunnerBroker {
    fn send(&self, _: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        panic!("navigation must consume its original stream")
    }
    fn start_operation(&self, request: NetworkRequest) -> Result<NetworkOperation, NetworkError> {
        let (sender, events) = tokio::sync::mpsc::channel(4);
        sender
            .try_send(NetworkEvent::Headers {
                response_policy: None,
                timing: None,
                request_id: request.request_id,
                status: 200,
                headers: vec![("content-type".into(), "text/html".into())],
                content_length: None,
                upload: None,
            })
            .unwrap();
        // The upstream runner has no beforeunload handler. Preserve its inline
        // bootstrap globals and apply runner.js's title change after first load.
        for bytes in [
            b"<!doctype html><title></title>".as_slice(),
            b"<script>var timeout_multiplier = 1; var win = null;</script>".as_slice(),
        ] {
            sender
                .try_send(NetworkEvent::Chunk {
                    request_id: request.request_id,
                    bytes: bytes::Bytes::copy_from_slice(bytes),
                })
                .unwrap();
        }
        sender
            .try_send(NetworkEvent::Complete {
                request_id: request.request_id,
                encoded_body_size: None,
            })
            .unwrap();
        Ok(NetworkOperation {
            body_demand_pending: Default::default(),
            prefetched: Default::default(),
            request_id: request.request_id,
            events,
            demand: None,
        })
    }
}

#[test]
fn sequential_cross_origin_streamed_runner_navigation_settles() {
    let mut page = page(Some(SharedNetworkBroker::new(RunnerBroker)));
    let mut generation = page.document_generation;
    for url in [
        "http://runner.test/testharness_runner.html",
        "https://runner.test/testharness_runner.html",
    ] {
        let result = page
            .navigate_resource(url.into(), HistoryUpdate::Push, None, None, None, None)
            .unwrap();
        assert_eq!(result.final_url, url);
        assert_eq!(result.status, 200);
        assert!(result.document_generation > generation);
        assert!(result.lifecycle.contains(&LifecycleState::Load));
        assert_eq!(
            page.evaluate("String(timeout_multiplier === 1 && win === null)")
                .unwrap(),
            "true"
        );
        page.evaluate("document.title = 'WPT runner'").unwrap();
        assert_eq!(page.evaluate("document.title").unwrap(), "WPT runner");
        generation = result.document_generation;
    }
}

struct HeldBodyBroker(Arc<Mutex<Option<(u64, tokio::sync::mpsc::Sender<NetworkEvent>)>>>);
impl NetworkBroker for HeldBodyBroker {
    fn send(&self, _: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        panic!("must use the held navigation stream")
    }
    fn start_operation(&self, request: NetworkRequest) -> Result<NetworkOperation, NetworkError> {
        let (sender, events) = tokio::sync::mpsc::channel(4);
        sender
            .try_send(NetworkEvent::Headers {
                response_policy: None,
                timing: None,
                request_id: request.request_id,
                status: 200,
                headers: vec![("content-type".into(), "text/html; charset=utf-8".into())],
                content_length: None,
                upload: None,
            })
            .unwrap();
        let html = format!(
            "<!doctype html><title>Streaming</title><p>body</p><!--{}-->",
            "x".repeat(2048)
        );
        sender
            .try_send(NetworkEvent::Chunk {
                request_id: request.request_id,
                bytes: bytes::Bytes::from(html),
            })
            .unwrap();
        *self.0.lock().unwrap() = Some((request.request_id, sender));
        Ok(NetworkOperation {
            body_demand_pending: Default::default(),
            prefetched: Default::default(),
            request_id: request.request_id,
            events,
            demand: None,
        })
    }
}

#[test]
fn actual_stream_failure_reaches_delayed_reply_through_page_task_poll() {
    let held = Arc::new(Mutex::new(None));
    let mut page = page(Some(SharedNetworkBroker::new(HeldBodyBroker(held.clone()))));
    let source_generation = page.document_generation;
    page.begin_navigation_resource(
        "https://runner.test/held".into(),
        HistoryUpdate::Push,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while page.document_generation == source_generation {
        page.poll_navigation_transaction();
        page.poll_page_tasks().unwrap();
        assert!(
            Instant::now() < deadline,
            "held response must commit before its terminal event"
        );
        std::thread::yield_now();
    }
    assert!(!page.executable.as_ref().unwrap().document_load_complete());
    page.active_navigation = Some(emitter());
    let mut reply = delayed_reply(&mut page, None);
    let (request_id, sender) = held.lock().unwrap().take().unwrap();
    sender
        .try_send(NetworkEvent::Failed {
            request_id,
            error: crate::network::NetworkFailure::Message("injected streamed body failure".into()),
        })
        .unwrap();
    let error = loop {
        match page.poll_page_tasks() {
            Err(error) => break error,
            Ok(()) => assert!(
                Instant::now() < deadline,
                "terminal body failure must settle"
            ),
        }
        std::thread::yield_now();
    };
    assert!(
        error.to_string().contains("injected streamed body failure"),
        "{error:?}"
    );
    assert_eq!(reply.try_recv().unwrap().unwrap_err(), error);
    assert!(page.active_navigation.is_none());
    assert!(page.settling_navigation.is_none());
    assert!(sender.is_closed(), "failed stream custody must be released");
}

#[test]
fn explicit_stop_retains_cancellation_without_inventing_beforeunload() {
    let mut page = page(None);
    let mut reply = delayed_reply(&mut page, None);
    page.stop_navigation_load().unwrap();
    let error = reply.try_recv().unwrap().unwrap_err();
    assert_eq!(error, RuntimeError::NavigationCancelled);
    assert_eq!(error.to_string(), "navigation cancelled");
}

#[test]
fn retained_final_ack_preserves_typed_stop_and_existing_failure() {
    for reason in [
        NavigationStop::Timeout,
        NavigationStop::Retired,
        NavigationStop::CertificateRejected("final acknowledgement certificate".into()),
    ] {
        for first in [
            None,
            Some(RuntimeError::Script("earlier parser failure".into())),
        ] {
            let mut page = page(None);
            let mut reply = delayed_reply(&mut page, first.clone());
            assert!(page.poll_navigation_transaction().is_none());
            assert!(matches!(
                reply.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            ));
            let settlement = page.settling_navigation.as_mut().unwrap();
            if reason == NavigationStop::Timeout {
                settlement.deadline = Instant::now();
            } else {
                settlement
                    .ack
                    .as_ref()
                    .unwrap()
                    .test_finish(Err(reason.clone()));
            }
            assert!(page.poll_navigation_transaction().is_none());
            let error = reply.try_recv().unwrap().unwrap_err();
            if let Some(first) = first {
                assert_eq!(error, first);
            } else {
                assert_eq!(error.navigation_stop(), reason);
            }
            assert!(page.settling_navigation.is_none());
        }
    }
}
