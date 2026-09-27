//! Real actor transaction tests: no page-task pump between ready HTTP handoffs.
use super::*;
use crate::network::{NetworkError, NetworkEvent, NetworkOperation, NetworkResponse};
use std::sync::atomic::{AtomicUsize, Ordering};

struct ReadyBroker(Arc<AtomicUsize>);
impl NetworkBroker for ReadyBroker {
    fn send(&self, _: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        panic!("the actor must retain the original operation")
    }
    fn start_operation(&self, request: NetworkRequest) -> Result<NetworkOperation, NetworkError> {
        self.0.fetch_add(1, Ordering::SeqCst);
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
        sender
            .try_send(NetworkEvent::Chunk {
                request_id: request.request_id,
                bytes: bytes::Bytes::from_static(b"<title>Ready destination</title>"),
            })
            .unwrap();
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

fn fixture() -> (
    PageState,
    NavigationTransaction,
    tokio::sync::mpsc::Sender<NetworkEvent>,
    Arc<AtomicUsize>,
) {
    let calls = Arc::new(AtomicUsize::new(0));
    let broker = SharedNetworkBroker::new(ReadyBroker(calls.clone()));
    let mut page = PageState::new(
        PageId(941_081),
        None,
        BrowserConfig::default(),
        ContextOptions::default(),
        Some(broker.clone()),
        #[cfg(unix)]
        None,
    );
    page.load_document(
        "https://ready.test/source".into(),
        "<title>Source</title>",
        None,
        None,
        HistoryUpdate::Push,
        &BTreeMap::new(),
        Vec::new(),
    )
    .unwrap();
    page.begin_navigation_resource(
        "https://ready.test/destination".into(),
        HistoryUpdate::Push,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let mut transaction = page.pending_navigation_transaction.take().unwrap();
    let (sender, events) = tokio::sync::mpsc::channel(4);
    let operation = NetworkOperation {
        body_demand_pending: Default::default(),
        prefetched: Default::default(),
        request_id: 19,
        events,
        demand: None,
    };
    let response = navigation_http::Response {
        #[cfg(unix)]
        response_policy_cleanup: None,
        timing: oryn_js::navigation_timing::NavigationTiming::new(
            Instant::now(),
            oryn_js::navigation_timing::Kind::Navigate,
        ),
        request_state: NavigationRequestState {
            initiator_url: Some(page.url.clone()),
            opaque_initiator: false,
            referrer: Some(page.url.clone()),
            referrer_policy: "strict-origin-when-cross-origin".into(),
        },
        broker,
        current: Url::parse("https://ready.test/destination").unwrap(),
        post: None,
        cross_origin_redirect: false,
        headers: (200, Vec::new(), None, Instant::now(), Instant::now(), None),
    };
    transaction.phase = NavigationTransactionPhase::Mime {
        pending: crate::network::DocumentMimeSniff::new(operation, None, false),
        response,
    };
    (page, transaction, sender, calls)
}
fn complete(sender: &tokio::sync::mpsc::Sender<NetworkEvent>) {
    sender
        .try_send(NetworkEvent::Chunk {
            request_id: 19,
            bytes: bytes::Bytes::from_static(b"<title>Ready destination</title>"),
        })
        .unwrap();
    sender
        .try_send(NetworkEvent::Complete {
            request_id: 19,
            encoded_body_size: None,
        })
        .unwrap();
}
fn assert_departure_entered(
    page: &PageState,
    transaction: &NavigationTransaction,
    progress: &std::task::Poll<Result<NavigationResult, RuntimeError>>,
) {
    assert!(
        !matches!(transaction.phase, NavigationTransactionPhase::Mime { .. }),
        "ready MIME must not consume another page-task turn"
    );
    if let NavigationTransactionPhase::Departure { job, retry, .. } = &transaction.phase {
        assert!(
            job.is_some() && retry.is_some(),
            "departure was actually polled, rather than only staged"
        );
    } else {
        assert!(matches!(progress, std::task::Poll::Ready(Ok(_))));
        assert_eq!(page.url, "https://ready.test/destination");
    }
}

#[test]
fn ready_actor_mime_enters_departure_without_another_task_turn() {
    let (mut page, mut transaction, sender, calls) = fixture();
    complete(&sender);
    let progress = page.step_ready_navigation_transaction(&mut transaction);
    assert_departure_entered(&page, &transaction, &progress);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "handed-off body must not be fetched again"
    );
}

#[test]
fn ready_actor_pending_mime_yields_and_preserves_the_incumbent() {
    let (mut page, mut transaction, sender, calls) = fixture();
    let generation = page.document_generation;
    for _ in 0..3 {
        assert!(
            page.step_ready_navigation_transaction(&mut transaction)
                .is_pending()
        );
        assert!(matches!(
            transaction.phase,
            NavigationTransactionPhase::Mime { .. }
        ));
        assert_eq!(page.document_generation, generation);
        assert_eq!(page.url, "https://ready.test/source");
        assert!(!sender.is_closed());
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    complete(&sender);
    let progress = page.step_ready_navigation_transaction(&mut transaction);
    assert_departure_entered(&page, &transaction, &progress);
}

#[test]
fn ready_actor_deadline_replacement_and_closed_reply_prevent_departure() {
    for outcome in ["deadline", "replacement", "closed"] {
        let (mut page, mut transaction, sender, calls) = fixture();
        complete(&sender);
        match outcome {
            "deadline" => transaction.deadline = Instant::now(),
            "replacement" => page.document_generation += 1,
            "closed" => {
                let (reply, received) = oneshot::channel();
                drop(received);
                transaction.reply = Some(NavigationReply::Html(reply));
            }
            _ => unreachable!(),
        }
        assert!(matches!(
            page.step_ready_navigation_transaction(&mut transaction),
            std::task::Poll::Ready(Err(_))
        ));
        assert!(
            matches!(transaction.phase, NavigationTransactionPhase::Mime { .. }),
            "failed liveness must not consume the ready response"
        );
        assert_eq!(page.url, "https://ready.test/source");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        drop(transaction);
        assert!(
            sender.is_closed(),
            "failed owner releases retained body custody"
        );
    }
}

#[test]
fn ready_actor_retained_ack_blocks_ready_mime_and_preserves_rejection() {
    use crate::navigation_control::{
        NavigationCommand, NavigationControl, NavigationEmitter, NavigationRequest, NavigationStop,
        PageLoadStrategy, PendingNavigationAck,
    };
    for reject in [false, true] {
        let (mut page, mut transaction, sender, calls) = fixture();
        complete(&sender);
        let control = NavigationControl::default();
        let job = control
            .begin(NavigationRequest {
                command: NavigationCommand::Navigate {
                    url: "https://ready.test/destination".into(),
                },
                strategy: PageLoadStrategy::Normal,
                timeout_ms: None,
            })
            .unwrap();
        job.publish(crate::navigation_control::NavigationEvent::Started)
            .unwrap();
        transaction.emitter = Some(NavigationEmitter::Local(job));
        let ack = PendingNavigationAck::test_pending();
        transaction.checkpoint = Some(ack.clone());
        assert!(
            page.step_ready_navigation_transaction(&mut transaction)
                .is_pending()
        );
        assert!(transaction.checkpoint.is_some());
        assert!(matches!(
            transaction.phase,
            NavigationTransactionPhase::Mime { .. }
        ));
        assert_eq!(page.url, "https://ready.test/source");
        ack.test_finish(if reject {
            Err(NavigationStop::Retired)
        } else {
            Ok(())
        });
        let progress = page.step_ready_navigation_transaction(&mut transaction);
        if reject {
            assert!(matches!(
                progress,
                std::task::Poll::Ready(Err(RuntimeError::Network(_)))
            ));
            assert!(matches!(
                transaction.phase,
                NavigationTransactionPhase::Mime { .. }
            ));
        } else {
            // The following step rechecks the live emitter instead of reusing
            // the previous ACK. The local emitter makes that new ACK ready.
            assert_departure_entered(&page, &transaction, &progress);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn ready_actor_http_headers_cross_mime_once_without_replaying_request() {
    let (mut page, mut transaction, sender, calls) = fixture();
    let broker = page.broker.clone().unwrap();
    transaction.phase =
        NavigationTransactionPhase::Http(Box::new(navigation_http::HttpNavigation::new(
            broker,
            #[cfg(unix)]
            None,
            page.page_id,
            #[cfg(unix)]
            crate::context_service::NavigationPolicySource::InitialAboutBlank,
            Url::parse("https://ready.test/destination").unwrap(),
            None,
            None,
            transaction.cookie_context.clone(),
            None,
            crate::referrer::ReferrerPolicy::default(),
            10,
            Some(transaction.deadline),
        )));
    assert!(sender.is_closed());
    let limit = Instant::now() + Duration::from_secs(5);
    loop {
        let progress = page.step_ready_navigation_transaction(&mut transaction);
        if !matches!(transaction.phase, NavigationTransactionPhase::Http(_)) {
            assert_departure_entered(&page, &transaction, &progress);
            break;
        }
        assert!(progress.is_pending());
        assert!(Instant::now() < limit, "owned IO admission must settle");
        std::thread::yield_now();
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
