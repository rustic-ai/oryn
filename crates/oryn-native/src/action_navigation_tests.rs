//! Regression coverage for the real action -> loader -> next command boundary.
//! The broker releases headers/body explicitly; no action or response is replayed.
use super::*;
use crate::network::{NetworkError, NetworkEvent, NetworkOperation, NetworkResponse};
use crate::security::PolicyRule;
use std::sync::atomic::AtomicUsize;

type Held = Arc<Mutex<Vec<(u64, tokio::sync::mpsc::Sender<NetworkEvent>)>>>;
struct Broker {
    held: Held,
    requests: Arc<Mutex<Vec<NetworkRequest>>>,
    started: tokio::sync::mpsc::UnboundedSender<()>,
    cancelled: Arc<AtomicUsize>,
}
fn headers(request_id: u64, status: u16, extra: Vec<(String, String)>) -> NetworkEvent {
    let mut headers = vec![("content-type".into(), "text/html".into())];
    headers.extend(extra);
    NetworkEvent::Headers {
        response_policy: None,
        timing: None,
        request_id,
        status,
        headers,
        content_length: None,
        upload: None,
    }
}
fn complete(request_id: u64) -> NetworkEvent {
    NetworkEvent::Complete {
        encoded_body_size: None,
        request_id,
    }
}
fn body(request_id: u64) -> NetworkEvent {
    NetworkEvent::Chunk {
        request_id,
        bytes:
            b"<!doctype html><title>Destination</title><label>Username<input id=username></label>"
                .to_vec()
                .into(),
    }
}
impl NetworkBroker for Broker {
    fn send(&self, _: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        unreachable!("navigation uses owned operations")
    }
    fn start_operation(&self, request: NetworkRequest) -> Result<NetworkOperation, NetworkError> {
        let (sender, events) = tokio::sync::mpsc::channel(8);
        let id = request.request_id;
        let held = request.url.ends_with("/submit");
        let child = request.url.ends_with("/child")
            || request.url.ends_with("/child-top")
            || request.url.ends_with("/child-history");
        let top = request.url.ends_with("/child-top");
        let history = request.url.ends_with("/child-history");
        self.requests.lock().unwrap().push(request);
        if held {
            self.held.lock().unwrap().push((id, sender));
            let _ = self.started.send(());
        } else {
            sender.try_send(headers(id, 200, Vec::new())).unwrap();
            if child {
                let target = if top { " target=_top" } else { "" };
                let source = if history {
                    "<!doctype html><title>History child</title><button id=back onclick='top.history.back()'>Back</button>".to_string()
                } else {
                    format!(
                        "<!doctype html><title>Child source</title><form method=post action=/submit{target}><input name=x value=owned><button id=submit>Continue</button></form>"
                    )
                };
                sender
                    .try_send(NetworkEvent::Chunk {
                        request_id: id,
                        bytes: source.into_bytes().into(),
                    })
                    .unwrap();
            } else {
                sender.try_send(body(id)).unwrap();
            }
            sender.try_send(complete(id)).unwrap();
        }
        Ok(NetworkOperation {
            body_demand_pending: Default::default(),
            prefetched: Default::default(),
            request_id: id,
            events,
            demand: None,
        })
    }
    fn cancel_operation(&self, _: u64) -> Result<(), NetworkError> {
        self.cancelled.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
struct Fixture {
    context: BrowserContext,
    page: PageHandle,
    held: Held,
    requests: Arc<Mutex<Vec<NetworkRequest>>>,
    started: tokio::sync::mpsc::UnboundedReceiver<()>,
    cancelled: Arc<AtomicUsize>,
    frame_source_generation: u64,
}
impl Fixture {
    async fn new(config: BrowserConfig) -> Self {
        let held = Arc::new(Mutex::new(Vec::new()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let cancelled = Arc::new(AtomicUsize::new(0));
        let (started, receiver) = tokio::sync::mpsc::unbounded_channel();
        let mut options = ContextOptions::default();
        options
            .action_policy
            .rules
            .insert(ConsequentialAction::FormSubmission, PolicyRule::Allow);
        let context = Browser::in_process_with_config(config).new_context_with_broker(
            options,
            SharedNetworkBroker::new(Broker {
                held: held.clone(),
                requests: requests.clone(),
                started,
                cancelled: cancelled.clone(),
            }),
        );
        let page = context.new_page();
        page.load_html("https://action-fence.test/source",
            "<!doctype html><title>Source</title><form method=post action=/submit><input name=x value=owned><button id=submit>Continue</button></form><input id=old value=source>")
            .await.unwrap();
        Self {
            context,
            page,
            held,
            requests,
            started: receiver,
            cancelled,
            frame_source_generation: 0,
        }
    }
    async fn click(&mut self) -> ActionResult {
        let target = self
            .page
            .resolve_selector("#submit", SemanticAction::Click)
            .await
            .unwrap();
        let action = tokio::time::timeout(
            Duration::from_secs(5),
            self.page.execute(NativeAction {
                target,
                action: SemanticAction::Click,
                value: None,
            }),
        )
        .await
        .expect("action admission must not await held headers")
        .unwrap();
        tokio::time::timeout(Duration::from_secs(5), self.started.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            action
                .effects
                .iter()
                .any(|effect| matches!(effect, Effect::Navigation { url }
            if url == "https://action-fence.test/submit"))
        );
        assert_eq!(self.requests.lock().unwrap().len(), 1);
        action
    }
    fn release(&self, status: u16) {
        let (id, sender) = self.held.lock().unwrap()[0].clone();
        let extra = if status == 200 {
            Vec::new()
        } else {
            vec![("location".into(), "/landed".into())]
        };
        sender.try_send(headers(id, status, extra)).unwrap();
        if status == 200 {
            sender.try_send(body(id)).unwrap();
        }
        sender.try_send(complete(id)).unwrap();
    }
    fn fail(&self) {
        let (id, sender) = self.held.lock().unwrap()[0].clone();
        sender
            .try_send(NetworkEvent::Failed {
                request_id: id,
                error: "owned response rejected".into(),
            })
            .unwrap();
    }
}

#[tokio::test(flavor = "current_thread")]
async fn admitted_action_fences_observation_until_destination_load() {
    let mut f = Fixture::new(Default::default()).await;
    let before = f.page.observe().await.unwrap().page.document_generation;
    let action = f.click().await;
    let mut observation = Box::pin(f.page.observe());
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut observation)
            .await
            .is_err(),
        "old source observation must not escape the admitted action fence"
    );
    // A different root remains schedulable while the first owns a retained command.
    let sibling = f.context.new_page();
    sibling
        .load_html(
            "https://action-fence.test/sibling",
            "<title>Sibling</title>",
        )
        .await
        .unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), sibling.evaluate("6*7"))
            .await
            .unwrap()
            .unwrap(),
        "42"
    );
    f.release(200);
    let after = tokio::time::timeout(Duration::from_secs(5), observation)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(after.page.document_generation, before);
    assert_eq!(after.page.url, "https://action-fence.test/submit");
    assert_eq!(after.page.title, "Destination");
    let target = f
        .page
        .resolve_selector("#username", SemanticAction::Type)
        .await
        .unwrap();
    let typed = f
        .page
        .execute(NativeAction {
            target,
            action: SemanticAction::Type,
            value: Some("alice".into()),
        })
        .await
        .unwrap();
    assert_eq!(
        typed.action_id.0,
        action.action_id.0 + 1,
        "only the original action consumed its identity"
    );
    assert_eq!(
        f.page
            .evaluate("document.getElementById('username').value")
            .await
            .unwrap(),
        "alice"
    );
    let requests = f.requests.lock().unwrap();
    assert_eq!(requests.len(), 1, "no duplicate form submission");
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].body, b"x=owned");
    drop(requests);
    sibling.close().await.unwrap();
    f.page.close().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn action_fence_waits_for_body_completion_not_just_headers_or_commit() {
    let mut f = Fixture::new(Default::default()).await;
    f.click().await;
    let (id, sender) = f.held.lock().unwrap()[0].clone();
    sender.try_send(headers(id, 200, Vec::new())).unwrap();
    sender.try_send(body(id)).unwrap();
    let mut selected = Box::pin(f.page.resolve_selector("#username", SemanticAction::Type));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut selected)
            .await
            .is_err()
    );
    sender.try_send(complete(id)).unwrap();
    tokio::time::timeout(Duration::from_secs(5), selected)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(f.requests.lock().unwrap().len(), 1);
    f.page.close().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn action_fence_follows_303_and_307_without_replaying_submission() {
    for status in [303, 307] {
        let mut f = Fixture::new(Default::default()).await;
        f.click().await;
        let mut observation = Box::pin(f.page.observe());
        assert!(
            tokio::time::timeout(Duration::from_millis(30), &mut observation)
                .await
                .is_err()
        );
        f.release(status);
        let after = tokio::time::timeout(Duration::from_secs(5), observation)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.page.url, "https://action-fence.test/landed");
        let requests = f.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].method, "POST");
        assert_eq!(requests[0].body, b"x=owned");
        assert_eq!(
            requests[1].method,
            if status == 303 { "GET" } else { "POST" }
        );
        assert_eq!(
            requests[1].body.as_slice(),
            if status == 303 {
                &[][..]
            } else {
                &b"x=owned"[..]
            }
        );
        drop(requests);
        f.page.close().await.unwrap();
    }
}

#[tokio::test(flavor = "current_thread")]
async fn failed_action_navigation_preserves_action_identity_and_original_failure_until_recovery() {
    let mut f = Fixture::new(Default::default()).await;
    let action = f.click().await;
    f.fail();
    let error = tokio::time::timeout(
        Duration::from_secs(5),
        f.page.resolve_selector("#old", SemanticAction::Type),
    )
    .await
    .unwrap()
    .unwrap_err();
    assert!(
        matches!(&error, RuntimeError::ActionExecutionFailed { action_id, .. } if *action_id == action.action_id)
    );
    assert!(
        error
            .root_cause()
            .to_string()
            .contains("owned response rejected"),
        "{error}"
    );
    assert_eq!(
        f.page.observe().await.unwrap_err(),
        error,
        "retry must not fall back to the old document"
    );
    assert_eq!(f.requests.lock().unwrap().len(), 1);
    f.page
        .goto("https://action-fence.test/recovery")
        .await
        .unwrap();
    assert_eq!(f.page.observe().await.unwrap().page.title, "Destination");
    f.page.close().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn action_navigation_uses_its_existing_absolute_navigation_deadline() {
    let mut f = Fixture::new(BrowserConfig {
        navigation_timeout: Duration::from_millis(150),
        ..Default::default()
    })
    .await;
    let action = f.click().await;
    let error = tokio::time::timeout(Duration::from_secs(5), f.page.observe())
        .await
        .unwrap()
        .unwrap_err();
    assert!(
        matches!(&error, RuntimeError::ActionExecutionFailed { action_id, .. } if *action_id == action.action_id)
    );
    assert!(
        matches!(error.root_cause(), RuntimeError::WorkerLimitExceeded { .. }),
        "{error}"
    );
    assert_eq!(f.requests.lock().unwrap().len(), 1);
    f.page.close().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn explicit_close_cancels_retained_document_command_without_replaying_action() {
    let mut f = Fixture::new(Default::default()).await;
    f.click().await;
    let mut observation = Box::pin(f.page.observe());
    assert!(
        tokio::time::timeout(Duration::from_millis(30), &mut observation)
            .await
            .is_err()
    );
    tokio::time::timeout(Duration::from_secs(5), f.page.close())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(observation.await, Err(RuntimeError::Closed)));
    assert_eq!(f.requests.lock().unwrap().len(), 1);
    assert!(
        f.cancelled.load(Ordering::SeqCst) > 0,
        "owned transport receives cancellation"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn explicit_replacement_navigation_retires_old_action_fence_and_ignores_late_headers() {
    let mut f = Fixture::new(Default::default()).await;
    f.click().await;
    f.page
        .goto("https://action-fence.test/recovery")
        .await
        .unwrap();
    let (id, sender) = f.held.lock().unwrap()[0].clone();
    let _ = sender.try_send(headers(id, 200, Vec::new()));
    let _ = sender.try_send(body(id));
    let _ = sender.try_send(complete(id));
    let after = f.page.observe().await.unwrap();
    assert_eq!(after.page.url, "https://action-fence.test/recovery");
    assert_eq!(f.requests.lock().unwrap().len(), 2);
    f.page.close().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn cancelled_followup_does_not_cancel_or_replay_the_acknowledged_action() {
    let mut f = Fixture::new(Default::default()).await;
    f.click().await;
    let mut observation = Box::pin(f.page.observe());
    assert!(
        tokio::time::timeout(Duration::from_millis(30), &mut observation)
            .await
            .is_err()
    );
    drop(observation);
    f.release(200);
    let next = tokio::time::timeout(Duration::from_secs(5), f.page.observe())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(next.page.title, "Destination");
    assert_eq!(f.requests.lock().unwrap().len(), 1);
    f.page.close().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn explicit_wait_deadline_expires_while_navigation_fence_is_pending() {
    let mut f = Fixture::new(Default::default()).await;
    f.click().await;
    let wait = f.page.wait_for_timeout(
        WaitPredicate::SelectorVisible("#old".into()),
        Some(Duration::from_millis(30)),
    );
    let error = tokio::time::timeout(Duration::from_secs(2), wait)
        .await
        .unwrap()
        .unwrap_err();
    assert!(matches!(error, RuntimeError::WaitUnsatisfied(_)));
    f.release(200);
    assert_eq!(f.page.observe().await.unwrap().page.title, "Destination");
    assert_eq!(f.requests.lock().unwrap().len(), 1);
    f.page.close().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn same_document_action_does_not_wait_for_a_nonexistent_new_document() {
    let context = Browser::in_process_with_config(Default::default()).new_context();
    let page = context.new_page();
    page.load_html(
        "https://action-fence.test/source",
        "<a id=link href='#next'>Continue</a>",
    )
    .await
    .unwrap();
    let before = page.observe().await.unwrap().page.document_generation;
    let target = page
        .resolve_selector("#link", SemanticAction::Click)
        .await
        .unwrap();
    page.execute(NativeAction {
        target,
        action: SemanticAction::Click,
        value: None,
    })
    .await
    .unwrap();
    let after = tokio::time::timeout(Duration::from_secs(2), page.observe())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.page.document_generation, before);
    assert_eq!(after.page.url, "https://action-fence.test/source#next");
    page.close().await.unwrap();
}

#[test]
fn action_navigation_reply_preserves_existing_execution_observer_and_typed_cancellation() {
    let mut state = PageState::new(
        PageId(901),
        None,
        Default::default(),
        Default::default(),
        None,
        #[cfg(unix)]
        None,
    );
    let outcomes = Arc::new(Mutex::new(Vec::new()));
    let saved = outcomes.clone();
    let observer = oryn_js::navigation::NavigationExecutionObserver::new(move |outcome| {
        saved.lock().unwrap().push(outcome);
    });
    let (completion, mut response) = oneshot::channel();
    let reply = NavigationReply::ActionNavigation {
        completion,
        previous: Some(Box::new(NavigationReply::Execution(
            observer.clone(),
            state.document_generation,
        ))),
    };
    assert!(!reply.is_closed());
    reply.complete(&mut state, Err(RuntimeError::NavigationCancelled));
    assert_eq!(
        response.try_recv().unwrap(),
        Err(RuntimeError::NavigationCancelled)
    );
    assert_eq!(
        *outcomes.lock().unwrap(),
        [oryn_js::navigation::NavigationExecutionOutcome::Canceled]
    );
    observer.complete(oryn_js::navigation::NavigationExecutionOutcome::Failed {
        message: "late".into(),
    });
    assert_eq!(
        outcomes.lock().unwrap().len(),
        1,
        "existing execution observer still settles only once"
    );
}

#[test]
fn failed_action_fence_keeps_original_scheduler_error_and_action_identity() {
    let mut state = PageState::new(
        PageId(902),
        None,
        Default::default(),
        Default::default(),
        None,
        #[cfg(unix)]
        None,
    );
    let result = NavigationResult {
        page_id: state.page_id,
        requested_url: "about:blank".into(),
        final_url: "about:blank".into(),
        title: String::new(),
        status: 200,
        document_generation: state.document_generation,
        revision: Revision(state.revision),
        lifecycle: Vec::new(),
        challenge: Default::default(),
    };
    state
        .retain_action_navigation(
            ActionId(71),
            state.document_generation,
            Instant::now().checked_add(Duration::from_secs(1)),
            NavigationProgress::Ready(result),
        )
        .unwrap();
    let cause = RuntimeError::WorkerLimitExceeded {
        resource: "native callback budget".into(),
    };
    state.fail_action_navigation(cause.clone());
    let expected = RuntimeError::ActionExecutionFailed {
        action_id: ActionId(71),
        cause: Box::new(cause),
    };
    assert_eq!(
        state.poll_action_navigation(),
        std::task::Poll::Ready(Err(expected.clone()))
    );
    assert_eq!(
        state.poll_action_navigation(),
        std::task::Poll::Ready(Err(expected))
    );
}

impl Fixture {
    async fn frame_action(&mut self, target_top: bool) -> ActionResult {
        use crate::host::webdriver_script::{FrameCommand, FrameResponse};
        self.page
            .load_html(
                "https://action-fence.test/root",
                if target_top {
                    "<!doctype html><title>Root</title><iframe src=/child-top></iframe>"
                } else {
                    "<!doctype html><title>Root</title><iframe src=/child></iframe>"
                },
            )
            .await
            .unwrap();
        self.frame_source_generation = self.page.observe().await.unwrap().page.document_generation;
        let FrameResponse::Inventory(realms) = self
            .page
            .webdriver_frames(FrameCommand::Inventory)
            .await
            .unwrap()
        else {
            panic!("frame inventory")
        };
        let child = realms
            .into_iter()
            .find(|realm| realm.parent_realm.is_some() && realm.live)
            .unwrap();
        self.requests.lock().unwrap().clear();
        let selected = self.page.with_frame_scope(Some(child));
        let target = selected
            .resolve_selector("#submit", SemanticAction::Click)
            .await
            .unwrap();
        let action = tokio::time::timeout(
            Duration::from_secs(5),
            selected.execute(NativeAction {
                target,
                action: SemanticAction::Click,
                value: None,
            }),
        )
        .await
        .unwrap()
        .unwrap();
        tokio::time::timeout(Duration::from_secs(5), self.started.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(self.requests.lock().unwrap().len(), 1);
        action
    }
}

#[tokio::test(flavor = "current_thread")]
async fn selected_frame_action_transfers_navigation_receipt_to_root_actor() {
    use crate::host::webdriver_script::{FrameCommand, FrameResponse};
    let mut f = Fixture::new(Default::default()).await;
    f.frame_action(false).await;
    let mut observation = Box::pin(f.page.observe());
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut observation)
            .await
            .is_err(),
        "root observation must wait for the selected frame's acknowledged navigation"
    );
    f.release(200);
    let after = tokio::time::timeout(Duration::from_secs(5), observation)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.page.url, "https://action-fence.test/root");
    let FrameResponse::Inventory(realms) = f
        .page
        .webdriver_frames(FrameCommand::Inventory)
        .await
        .unwrap()
    else {
        panic!("frame inventory")
    };
    let child = realms
        .into_iter()
        .find(|realm| realm.parent_realm.is_some() && realm.live)
        .unwrap();
    assert_eq!(child.url, "https://action-fence.test/submit");
    assert_eq!(
        f.page
            .with_frame_scope(Some(child))
            .evaluate("document.title")
            .await
            .unwrap(),
        "Destination"
    );
    let requests = f.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].body, b"x=owned");
    drop(requests);
    f.page.close().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn selected_frame_top_navigation_is_admitted_by_root_document_owner() {
    let mut f = Fixture::new(Default::default()).await;
    let action = f.frame_action(true).await;
    assert!(
        action
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::Navigation { url }
        if url == "https://action-fence.test/submit"))
    );
    let generation = f.frame_source_generation;
    let mut observation = Box::pin(f.page.observe());
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut observation)
            .await
            .is_err()
    );
    f.release(200);
    let after = tokio::time::timeout(Duration::from_secs(5), observation)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.page.url, "https://action-fence.test/submit");
    assert_eq!(after.page.title, "Destination");
    assert_ne!(after.page.document_generation, generation);
    assert_eq!(f.requests.lock().unwrap().len(), 1);
    f.page.close().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn failed_selected_frame_navigation_preserves_original_action_failure_at_owner() {
    let mut f = Fixture::new(Default::default()).await;
    let action = f.frame_action(false).await;
    f.fail();
    let error = tokio::time::timeout(Duration::from_secs(5), f.page.observe())
        .await
        .unwrap()
        .unwrap_err();
    assert!(
        matches!(&error, RuntimeError::ActionExecutionFailed { action_id, .. } if *action_id == action.action_id)
    );
    assert!(
        error.to_string().contains("owned response rejected"),
        "{error}"
    );
    assert_eq!(f.requests.lock().unwrap().len(), 1);
    f.page.close().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn action_history_back_fences_next_observation_until_native_restore() {
    let f = Fixture::new(Default::default()).await;
    let first = f.page.observe().await.unwrap().page.document_generation;
    f.page
        .load_html(
            "https://action-fence.test/second",
            "<title>Second</title><button id=back onclick='history.back()'>Back</button>",
        )
        .await
        .unwrap();
    let target = f
        .page
        .resolve_selector("#back", SemanticAction::Click)
        .await
        .unwrap();
    f.page
        .execute(NativeAction {
            target,
            action: SemanticAction::Click,
            value: None,
        })
        .await
        .unwrap();
    let restored = tokio::time::timeout(Duration::from_secs(5), f.page.observe())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(restored.page.url, "https://action-fence.test/source");
    assert_eq!(restored.page.title, "Source");
    assert_eq!(restored.page.document_generation, first);
    assert!(
        f.requests.lock().unwrap().is_empty(),
        "native cached traversal must not be replayed as network navigation"
    );
    f.page.close().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn selected_frame_top_history_preserves_native_joint_receipt_at_root_owner() {
    use crate::host::webdriver_script::{FrameCommand, FrameResponse};
    let f = Fixture::new(Default::default()).await;
    let first = f.page.observe().await.unwrap().page.document_generation;
    f.page
        .load_html(
            "https://action-fence.test/second",
            "<title>Second</title><iframe src=/child-history></iframe>",
        )
        .await
        .unwrap();
    let FrameResponse::Inventory(realms) = f
        .page
        .webdriver_frames(FrameCommand::Inventory)
        .await
        .unwrap()
    else {
        panic!("frame inventory")
    };
    let child = realms
        .into_iter()
        .find(|realm| realm.parent_realm.is_some() && realm.live)
        .unwrap();
    let selected = f.page.with_frame_scope(Some(child));
    let target = selected
        .resolve_selector("#back", SemanticAction::Click)
        .await
        .unwrap();
    selected
        .execute(NativeAction {
            target,
            action: SemanticAction::Click,
            value: None,
        })
        .await
        .unwrap();
    let restored = tokio::time::timeout(Duration::from_secs(5), f.page.observe())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(restored.page.url, "https://action-fence.test/source");
    assert_eq!(restored.page.document_generation, first);
    assert_eq!(
        f.requests.lock().unwrap().len(),
        1,
        "only the original iframe resource was fetched"
    );
    f.page.close().await.unwrap();
}

#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn selected_frame_batch_waits_at_navigation_before_rejecting_stale_suffix() {
    use crate::host::webdriver_script::{FrameCommand, FrameResponse};
    let mut f = Fixture::new(Default::default()).await;
    f.page
        .load_html(
            "https://action-fence.test/root",
            "<title>Root</title><iframe src=/child-top></iframe>",
        )
        .await
        .unwrap();
    let FrameResponse::Inventory(realms) = f
        .page
        .webdriver_frames(FrameCommand::Inventory)
        .await
        .unwrap()
    else {
        panic!("frame inventory")
    };
    let child = realms
        .into_iter()
        .find(|realm| realm.parent_realm.is_some() && realm.live)
        .unwrap();
    let selected = f.page.with_frame_scope(Some(child));
    let target = selected
        .resolve_selector("#submit", SemanticAction::Click)
        .await
        .unwrap();
    f.requests.lock().unwrap().clear();
    let click = || NativeAction {
        target,
        action: SemanticAction::Click,
        value: None,
    };
    let mut batch = Box::pin(selected.execute_batch(vec![click(), click()]));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut batch)
            .await
            .is_err(),
        "the second action must remain behind the first action's native navigation"
    );
    tokio::time::timeout(Duration::from_secs(5), f.started.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(f.requests.lock().unwrap().len(), 1);
    f.release(200);
    let error = tokio::time::timeout(Duration::from_secs(5), batch)
        .await
        .unwrap()
        .unwrap_err();
    let RuntimeError::BatchExecutionFailed {
        completed,
        failed_index,
        ..
    } = error
    else {
        panic!("batch must retain its completed prefix")
    };
    assert_eq!(failed_index, 1);
    assert_eq!(completed.len(), 1);
    assert_eq!(
        f.requests.lock().unwrap().len(),
        1,
        "stale suffix was not dispatched or replayed"
    );
    assert_eq!(f.page.observe().await.unwrap().page.title, "Destination");
    f.page.close().await.unwrap();
}

#[test]
fn history_task_receipt_is_pending_until_its_original_native_plan_commits() {
    let mut page = PageState::new(
        PageId(9902),
        None,
        Default::default(),
        Default::default(),
        None,
        #[cfg(unix)]
        None,
    );
    for (url, html) in [
        ("https://history-fence.test/one", "<title>One</title>"),
        ("https://history-fence.test/two", "<title>Two</title>"),
    ] {
        page.load_document(
            url.into(),
            html,
            None,
            None,
            HistoryUpdate::Push,
            &BTreeMap::new(),
            Vec::new(),
        )
        .unwrap();
    }
    page.executable
        .as_mut()
        .unwrap()
        .execute("history.back()")
        .unwrap();
    let task = page
        .executable
        .as_ref()
        .unwrap()
        .take_history_traversal()
        .unwrap()
        .unwrap();
    let Some(NavigationProgress::Joint(receipt, _)) =
        page.begin_action_history_navigation(task).unwrap()
    else {
        panic!("the same queued history task must return its native receipt")
    };
    assert!(
        receipt.poll().is_none(),
        "queue admission is not document commit"
    );
    let request = page
        .executable
        .as_ref()
        .unwrap()
        .take_location_navigation()
        .unwrap();
    assert_eq!(request.url, "https://history-fence.test/one");
    assert!(
        page.executable
            .as_ref()
            .unwrap()
            .take_history_traversal()
            .unwrap()
            .is_none(),
        "original task was acknowledged exactly once"
    );
    receipt.cancel();
    assert!(matches!(
        receipt.poll(),
        Some(Err(oryn_js::joint_history::JointTraversalFailure::Canceled))
    ));
}

fn generic_frame_fixture(
    page_id: u64,
) -> (
    PageState,
    crate::host::webdriver_script::AutomationRealmSnapshot,
) {
    let mut page = PageState::new(
        PageId(page_id),
        None,
        Default::default(),
        Default::default(),
        None,
        #[cfg(unix)]
        None,
    );
    page.load_document(
        "https://generic-frame.test/root".into(),
        r#"<title>Root</title><iframe srcdoc='<title>Child</title><p id=ready>Ready</p>'></iframe>"#,
        None,
        None,
        HistoryUpdate::Push,
        &BTreeMap::new(),
        Vec::new(),
    )
    .unwrap();
    page.poll_page_tasks().unwrap();
    let scope = page
        .executable
        .as_ref()
        .unwrap()
        .automation_realms()
        .into_iter()
        .find(|realm| realm.parent_realm.is_some() && realm.live)
        .unwrap();
    (page, scope)
}

#[test]
fn generic_scoped_frame_preserves_boolean_and_owned_optional_results() {
    let (mut page, scope) = generic_frame_fixture(9910);
    let ready: bool = page
        .with_scoped_frame(scope.clone(), false, None, false, true, |frame| {
            frame
                .executable
                .as_mut()
                .unwrap()
                .evaluate("document.getElementById('ready') !== null")
                .map(|value| value == "true")
                .map_err(map_host_error)
        })
        .unwrap();
    assert!(ready, "the boolean wait result comes from the selected DOM");
    let expected = vec!["owned return value".to_string()];
    let owned: Option<Vec<String>> = page
        .with_scoped_frame(scope, false, None, false, false, |_frame| {
            Ok(Some(expected.clone()))
        })
        .unwrap();
    assert_eq!(owned, Some(expected));
    assert!(
        page.executable
            .as_ref()
            .unwrap()
            .current_automation_scope()
            .is_none()
    );
    assert_eq!(
        page.executable
            .as_mut()
            .unwrap()
            .evaluate("document.title")
            .unwrap(),
        "Root"
    );
}

#[test]
fn generic_scoped_frame_error_preserves_original_failure_and_restores_root() {
    let (mut page, scope) = generic_frame_fixture(9911);
    let original = RuntimeError::Script("original scoped operation failure".into());
    let result: Result<Option<Vec<String>>, RuntimeError> =
        page.with_scoped_frame(scope, false, None, false, false, |_frame| {
            Err(original.clone())
        });
    assert_eq!(result.unwrap_err(), original);
    assert!(
        page.executable
            .as_ref()
            .unwrap()
            .current_automation_scope()
            .is_none()
    );
    assert_eq!(
        page.executable
            .as_mut()
            .unwrap()
            .evaluate("document.title")
            .unwrap(),
        "Root"
    );
}

struct ImmediateSelectedFrameBroker {
    status: u16,
    requests: Arc<Mutex<Vec<NetworkRequest>>>,
}
impl NetworkBroker for ImmediateSelectedFrameBroker {
    fn send(&self, _: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        unreachable!("selected frame navigation uses owned operations")
    }
    fn start_operation(&self, request: NetworkRequest) -> Result<NetworkOperation, NetworkError> {
        let (sender, events) = tokio::sync::mpsc::channel(8);
        let id = request.request_id;
        let child = request.url.ends_with("/child");
        let submitted = request.url.ends_with("/submit");
        let status = if submitted { self.status } else { 200 };
        let extra = if status == 302 {
            vec![("location".into(), "/landed".into())]
        } else {
            Vec::new()
        };
        self.requests.lock().unwrap().push(request);
        sender.try_send(headers(id, status, extra)).unwrap();
        let source = if child {
            "<!doctype html><title>Payment</title><form method=post action=/submit><input name=x value=owned><button id=submit>Authorize</button></form>"
        } else if status == 403 {
            "<!doctype html><title>Denied</title><body>origin_denied</body>"
        } else {
            "<!doctype html><title>Destination</title><body>Complete</body>"
        };
        if status != 302 {
            sender
                .try_send(NetworkEvent::Chunk {
                    request_id: id,
                    bytes: source.as_bytes().to_vec().into(),
                })
                .unwrap();
        }
        sender.try_send(complete(id)).unwrap();
        Ok(NetworkOperation {
            body_demand_pending: Default::default(),
            prefetched: Default::default(),
            request_id: id,
            events,
            demand: None,
        })
    }
}

async fn immediate_selected_frame_action(status: u16) {
    use crate::host::webdriver_script::{FrameCommand, FrameResponse};
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut options = ContextOptions::default();
    options
        .action_policy
        .rules
        .insert(ConsequentialAction::FormSubmission, PolicyRule::Allow);
    let context = Browser::in_process_with_config(Default::default()).new_context_with_broker(
        options,
        SharedNetworkBroker::new(ImmediateSelectedFrameBroker {
            status,
            requests: requests.clone(),
        }),
    );
    let page = context.new_page();
    page.load_html("https://merchant-handoff.test/root", "<!doctype html><title>Merchant</title><iframe src='https://payment-handoff.test/child'></iframe>").await.unwrap();
    let FrameResponse::Inventory(realms) = page
        .webdriver_frames(FrameCommand::Inventory)
        .await
        .unwrap()
    else {
        panic!("frame inventory")
    };
    let original = realms
        .into_iter()
        .find(|entry| entry.parent_realm.is_some() && entry.live)
        .unwrap();
    let selected = page.with_frame_scope(Some(original.clone()));
    let target = selected
        .resolve_selector("#submit", SemanticAction::Click)
        .await
        .unwrap();
    requests.lock().unwrap().clear();
    let action = tokio::time::timeout(
        Duration::from_secs(5),
        selected.execute(NativeAction {
            target,
            action: SemanticAction::Click,
            value: None,
        }),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(action.action_id, ActionId(1));
    assert!(action.effects.iter().any(|effect| matches!(effect, Effect::Event { event_type, target: recorded } if event_type == "click" && *recorded == target)));
    tokio::time::timeout(Duration::from_secs(5), page.observe())
        .await
        .unwrap()
        .unwrap();
    let FrameResponse::Inventory(realms) = page
        .webdriver_frames(FrameCommand::Inventory)
        .await
        .unwrap()
    else {
        panic!("frame inventory")
    };
    let replacement = realms
        .into_iter()
        .find(|entry| entry.same_navigable_as(&original) && entry.live)
        .unwrap();
    assert!(!replacement.same_document_as(&original));
    assert_eq!(
        replacement.url,
        if status == 302 {
            "https://payment-handoff.test/landed"
        } else {
            "https://payment-handoff.test/submit"
        }
    );
    assert_eq!(
        page.with_frame_scope(Some(replacement))
            .evaluate("document.title")
            .await
            .unwrap(),
        if status == 403 {
            "Denied"
        } else {
            "Destination"
        }
    );
    let stale = selected
        .execute(NativeAction {
            target,
            action: SemanticAction::Click,
            value: None,
        })
        .await;
    assert!(
        stale.is_err(),
        "original input capability cannot target the replacement"
    );
    let captured = requests.lock().unwrap();
    let posts = captured
        .iter()
        .filter(|request| request.method == "POST")
        .collect::<Vec<_>>();
    assert_eq!(
        posts.len(),
        1,
        "receipt/observation/stale retry never resubmits the form"
    );
    assert_eq!(posts[0].url, "https://payment-handoff.test/submit");
    assert_eq!(posts[0].body, b"x=owned");
    assert_eq!(captured.len(), if status == 302 { 2 } else { 1 });
    if status == 302 {
        assert_eq!(captured[1].method, "GET");
        assert!(captured[1].body.is_empty());
    }
    drop(captured);
    page.close().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn selected_action_handoff_preserves_receipt_for_immediate_child_post() {
    immediate_selected_frame_action(200).await;
}

#[tokio::test(flavor = "current_thread")]
async fn selected_action_handoff_preserves_receipt_for_committed_http_error_document() {
    immediate_selected_frame_action(403).await;
}

#[tokio::test(flavor = "current_thread")]
async fn selected_action_handoff_preserves_receipt_across_post_redirect_without_replay() {
    immediate_selected_frame_action(302).await;
}

#[tokio::test(flavor = "current_thread")]
async fn page_action_ids_interleave_root_nested_replacement_and_dispatched_error() {
    use crate::host::webdriver_script::{FrameCommand, FrameResponse};
    let context = Browser::in_process_with_config(Default::default()).new_context();
    let page = context.new_page();
    page.load_html("https://action-ids.test/", "<!doctype html><input id=root><iframe id=outer srcdoc=\"<input id=outer-input><iframe srcdoc='<input id=nested>'></iframe>\"></iframe>").await.unwrap();
    async fn focus(page: &PageHandle, selector: &str) -> ActionResult {
        let target = page
            .resolve_selector(selector, SemanticAction::Focus)
            .await
            .unwrap();
        page.execute(NativeAction {
            target,
            action: SemanticAction::Focus,
            value: None,
        })
        .await
        .unwrap()
    }
    let first = focus(&page, "#root").await;
    let FrameResponse::Inventory(frames) = page
        .webdriver_frames(FrameCommand::Inventory)
        .await
        .unwrap()
    else {
        panic!("inventory")
    };
    let outer = frames
        .iter()
        .find(|f| {
            f.live
                && f.parent_realm.is_some()
                && frames
                    .iter()
                    .any(|child| child.parent_realm == Some(f.realm))
        })
        .unwrap()
        .clone();
    let nested = frames
        .iter()
        .find(|f| f.live && f.parent_realm == Some(outer.realm))
        .unwrap()
        .clone();
    let selected = page.with_frame_scope(Some(outer.clone()));
    let second = focus(&selected, "#outer-input").await;
    let third = focus(&page.with_frame_scope(Some(nested)), "#nested").await;
    let fourth = focus(&page, "#root").await;
    assert_eq!(
        [
            first.action_id.0,
            second.action_id.0,
            third.action_id.0,
            fourth.action_id.0
        ],
        [1, 2, 3, 4]
    );
    assert_eq!(first.page_id, second.page_id);
    assert_eq!(first.page_id, third.page_id);
    selected.evaluate("document.getElementById('outer-input').addEventListener('beforeinput',e=>e.preventDefault());'ready'").await.unwrap();
    let target = selected
        .resolve_selector("#outer-input", SemanticAction::Type)
        .await
        .unwrap();
    let error = selected
        .execute(NativeAction {
            target,
            action: SemanticAction::Type,
            value: Some("blocked".into()),
        })
        .await
        .unwrap_err();
    let RuntimeError::ActionExecutionFailed { action_id, .. } = error else {
        panic!("dispatched error: {error:?}")
    };
    assert_eq!(action_id, ActionId(5));
    assert_eq!(focus(&page, "#root").await.action_id, ActionId(6));
    page.evaluate("document.getElementById('outer').srcdoc='<input id=replacement>'; 'changed'")
        .await
        .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let replacement = loop {
        let FrameResponse::Inventory(frames) = page
            .webdriver_frames(FrameCommand::Inventory)
            .await
            .unwrap()
        else {
            panic!("inventory")
        };
        if let Some(frame) = frames
            .into_iter()
            .find(|f| f.live && f.same_navigable_as(&outer) && !f.same_document_as(&outer))
        {
            break frame;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "replacement not committed"
        );
        page.observe().await.unwrap();
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    assert_eq!(
        focus(&page.with_frame_scope(Some(replacement)), "#replacement")
            .await
            .action_id,
        ActionId(7)
    );
    assert_eq!(focus(&page, "#root").await.action_id, ActionId(8));
    let target = page
        .resolve_selector("#root", SemanticAction::Type)
        .await
        .unwrap();
    assert_eq!(
        page.execute_sensitive(NativeAction {
            target,
            action: SemanticAction::Type,
            value: Some("private".into())
        })
        .await
        .unwrap()
        .action_id,
        ActionId(9)
    );
    let target = page
        .resolve_selector("#root", SemanticAction::Click)
        .await
        .unwrap();
    assert_eq!(
        page.execute_forced(NativeAction {
            target,
            action: SemanticAction::Click,
            value: None
        })
        .await
        .unwrap()
        .action_id,
        ActionId(10)
    );
    page.evaluate(
        "document.body.insertAdjacentHTML('beforeend','<input type=file id=upload>');'ready'",
    )
    .await
    .unwrap();
    let target = page
        .resolve_selector("#upload", SemanticAction::Click)
        .await
        .unwrap();
    assert_eq!(
        page.set_files_materialized(
            target,
            vec![crate::upload::UploadFile {
                filename: "owned.txt".into(),
                media_type: "text/plain".into(),
                bytes: b"owned".to_vec()
            }]
        )
        .await
        .unwrap()
        .action_id,
        ActionId(11)
    );
    assert_eq!(
        page.evaluate("upload.files[0].name + ':' + upload.files[0].size")
            .await
            .unwrap(),
        "owned.txt:5"
    );
    let independent = context.new_page();
    independent
        .load_html("https://action-ids.test/new", "<input id=root>")
        .await
        .unwrap();
    assert_eq!(focus(&independent, "#root").await.action_id, ActionId(1));
    independent.close().await.unwrap();
    page.close().await.unwrap();
}

#[test]
fn page_action_allocator_shared_exhaustion_never_wraps_or_reuses() {
    let root = PageActionIds::default();
    let frame = root.clone();
    let nested = frame.clone();
    root.set_last(u64::MAX - 1);
    assert_eq!(nested.reserve().unwrap(), ActionId(u64::MAX));
    for owner in [&root, &frame, &nested] {
        assert!(
            matches!(owner.reserve(), Err(RuntimeError::IdentityExhausted { namespace }) if namespace == "native action")
        );
        assert_eq!(owner.last(), u64::MAX);
    }
    assert_eq!(PageActionIds::default().reserve().unwrap(), ActionId(1));
}
