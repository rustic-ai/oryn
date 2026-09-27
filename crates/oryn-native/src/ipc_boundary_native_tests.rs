//! Real attested worker IPC failure boundaries. Include as a runtime child module.
//! Network events are native broker fixtures, not fabricated worker replies.
use super::*;
use crate::network::{NetworkError, NetworkEvent, NetworkOperation, NetworkResponse};
use std::collections::BTreeMap;
use std::sync::Condvar;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Notice {
    Navigation,
    Mutation,
    Fetch(u64),
    CancelEntered(u64),
}
fn witness(phase: &str, request_id: u64) {
    let unix_time_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    eprintln!(
        "{{\"kind\":\"oryn_ipc_boundary_fixture\",\"pid\":{},\"unix_time_ns\":{},\"request_id\":{},\"phase\":\"{}\"}}",
        std::process::id(),
        unix_time_ns,
        request_id,
        phase
    );
}
#[derive(Default)]
struct Gate {
    open: Mutex<bool>,
    changed: Condvar,
    entered: Mutex<Option<u64>>,
}
impl Gate {
    fn release(&self) {
        *self.open.lock().unwrap() = true;
        if let Some(id) = *self.entered.lock().unwrap() {
            witness("cancel_gate_released", id);
        }
        self.changed.notify_all();
    }
    fn wait(&self, id: u64, notices: &tokio::sync::mpsc::UnboundedSender<Notice>) {
        let mut open = self.open.lock().unwrap();
        if !*open {
            *self.entered.lock().unwrap() = Some(id);
            witness("cancel_gate_entered", id);
            let _ = notices.send(Notice::CancelEntered(id));
        }
        while !*open {
            open = self.changed.wait(open).unwrap();
        }
        witness("cancel_gate_exited", id);
    }
}
type Held = Arc<Mutex<BTreeMap<u64, tokio::sync::mpsc::Sender<NetworkEvent>>>>;
struct Broker {
    held: Held,
    notices: tokio::sync::mpsc::UnboundedSender<Notice>,
    mutations: Arc<AtomicUsize>,
    cancelled: Arc<AtomicUsize>,
    cancel_gate: Arc<Gate>,
    hold_cancel: Arc<AtomicBool>,
    navigation_headers: bool,
}
fn headers(id: u64) -> NetworkEvent {
    NetworkEvent::Headers {
        response_policy: None,
        timing: None,
        request_id: id,
        status: 200,
        headers: vec![
            ("content-type".into(), "text/html;charset=utf-8".into()),
            ("x-content-type-options".into(), "nosniff".into()),
        ],
        content_length: None,
        upload: None,
    }
}
impl NetworkBroker for Broker {
    fn send(&self, _: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        Err(NetworkError::Transport(
            "boundary fixture requires owned operation transport".into(),
        ))
    }
    fn start_operation(&self, request: NetworkRequest) -> Result<NetworkOperation, NetworkError> {
        let id = request.request_id;
        let (sender, events) = tokio::sync::mpsc::channel(8);
        if request.url.ends_with("/mutate") {
            assert_eq!(request.method, "POST");
            assert_eq!(request.body, b"once");
            self.mutations.fetch_add(1, Ordering::SeqCst);
            sender.try_send(headers(id)).unwrap();
            sender
                .try_send(NetworkEvent::Chunk {
                    request_id: id,
                    bytes: b"ok".to_vec().into(),
                })
                .unwrap();
            sender
                .try_send(NetworkEvent::Complete {
                    encoded_body_size: None,
                    request_id: id,
                })
                .unwrap();
            let _ = self.notices.send(Notice::Mutation);
        } else {
            let notice = if request.url.ends_with("/hold-navigation") {
                Notice::Navigation
            } else if request.url.ends_with("/hold-fetch") {
                Notice::Fetch(id)
            } else {
                return Err(NetworkError::Transport(
                    "unexpected boundary fixture URL".into(),
                ));
            };
            // Keep close fixtures pending before headers. An unconsumed Response
            // body is allowed to cancel before Close and cannot witness Close cleanup.
            if self.navigation_headers && notice == Notice::Navigation {
                sender.try_send(headers(id)).unwrap();
            }
            self.held.lock().unwrap().insert(id, sender);
            witness("request_held", id);
            let _ = self.notices.send(notice);
        }
        Ok(NetworkOperation {
            body_demand_pending: Default::default(),
            prefetched: Default::default(),
            request_id: id,
            events,
            demand: None,
        })
    }
    fn cancel_operation(&self, id: u64) -> Result<(), NetworkError> {
        if self.held.lock().unwrap().contains_key(&id) {
            self.cancelled.fetch_add(1, Ordering::SeqCst);
            witness("cancel_observed", id);
            if self.hold_cancel.load(Ordering::SeqCst) {
                self.cancel_gate.wait(id, &self.notices);
            }
            self.held.lock().unwrap().remove(&id);
        }
        Ok(())
    }
}
struct Fixture {
    held: Held,
    _context: BrowserContext,
    page: PageHandle,
    notices: tokio::sync::mpsc::UnboundedReceiver<Notice>,
    mutations: Arc<AtomicUsize>,
    cancelled: Arc<AtomicUsize>,
    cancel_gate: Arc<Gate>,
    hold_cancel: Arc<AtomicBool>,
    pid: u32,
}
impl Fixture {
    async fn new(navigation_headers: bool) -> Self {
        let (notices, receiver) = tokio::sync::mpsc::unbounded_channel();
        let mutations = Arc::new(AtomicUsize::new(0));
        let cancelled = Arc::new(AtomicUsize::new(0));
        let cancel_gate = Arc::new(Gate::default());
        let hold_cancel = Arc::new(AtomicBool::new(false));
        let held = Arc::new(Mutex::new(BTreeMap::new()));
        let broker = SharedNetworkBroker::new(Broker {
            held: held.clone(),
            notices,
            mutations: mutations.clone(),
            cancelled: cancelled.clone(),
            cancel_gate: cancel_gate.clone(),
            hold_cancel: hold_cancel.clone(),
            navigation_headers,
        });
        let mut browser = Browser::with_config(BrowserConfig {
            worker_lifecycle: WorkerLifecycle::IsolatedOnDemand,
            ..Default::default()
        });
        browser.worker_process = true; // Includes V8 test builds: always a real child.
        let context = browser.new_native_test_context_with_broker(Default::default(), broker);
        let page = context.new_page();
        page.load_html("https://ipc-boundary.test/source", "<!doctype html><button id=mutate onclick=\"var x=new XMLHttpRequest();x.open('POST','/mutate',false);x.send('once');for(;;){}\">Mutate</button>").await.unwrap();
        let pid = page
            .worker
            .as_ref()
            .unwrap()
            .process_id
            .load(Ordering::Acquire);
        assert_ne!(pid, 0);
        assert_ne!(pid, std::process::id());
        Self {
            held,
            _context: context,
            page,
            notices: receiver,
            mutations,
            cancelled,
            cancel_gate,
            hold_cancel,
            pid,
        }
    }
    async fn hold_fetch_for_close(&mut self) -> u64 {
        self.hold_cancel.store(true, Ordering::SeqCst);
        tokio::time::timeout(
            Duration::from_secs(10),
            self.page
                .evaluate("void (globalThis.heldFetch = fetch('/hold-fetch').catch(()=>{}))"),
        )
        .await
        .unwrap()
        .unwrap();
        let id = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Notice::Fetch(id) = self.notices.recv().await.expect("fetch notice") {
                    break id;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(
            self.cancelled.load(Ordering::SeqCst),
            0,
            "fetch cancelled before Close admission"
        );
        assert!(self.held.lock().unwrap().contains_key(&id));
        assert!(self.cancel_gate.entered.lock().unwrap().is_none());
        witness("close_gate_armed", id);
        id
    }
    fn budget(&self, duration: Duration) {
        self.page
            .worker
            .as_ref()
            .unwrap()
            .client
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .set_response_budget_for_test(duration);
    }
    async fn reached<F: std::future::Future>(
        &mut self,
        notice: Notice,
        work: &mut std::pin::Pin<Box<F>>,
    ) {
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                tokio::select! {
                    event = self.notices.recv() => {
                        let event = event.expect("broker notice channel closed");
                        if event == notice { break; }
                    }
                    _ = &mut *work => panic!("native operation settled before reaching {notice:?}"),
                }
            }
        })
        .await
        .expect("native boundary not reached");
    }
    fn reaped(&self) {
        assert!(
            self.held.lock().unwrap().is_empty(),
            "native request custody remains held"
        );
        let worker = self.page.worker.as_ref().unwrap();
        assert_eq!(worker.process_id.load(Ordering::Acquire), 0);
        assert!(worker.client.lock().unwrap().is_none());
        let mut status = 0;
        // SAFETY: only querying our original owned child. ECHILD proves its wait
        // custody has already been consumed; no signal or fabricated receipt.
        assert_eq!(
            unsafe { libc::waitpid(self.pid as libc::pid_t, &mut status, libc::WNOHANG) },
            -1
        );
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
    }
    async fn closed_without_replay(&self) {
        self.page.close().await.unwrap();
        assert!(matches!(
            self.page.evaluate("1").await,
            Err(RuntimeError::Closed)
        ));
        self.reaped();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // Even failed assertions release the real native cleanup barrier before
        // interrupt+join. No background request or process is detached by tests.
        self.cancel_gate.release();
        self.page.worker.as_ref().unwrap().retire();
    }
}
fn ipc_deadline(error: &RuntimeError, command: &str) {
    assert!(
        matches!(error, RuntimeError::WorkerLimitExceeded { .. }),
        "{error:?}"
    );
    let text = error.to_string();
    assert!(text.contains("phase=ipc_response"), "{text}");
    assert!(text.contains(&format!("command={command}")), "{text}");
}

#[tokio::test(flavor = "current_thread")]
async fn real_worker_navigation_deadline_cancels_headers_and_body_then_reaps() {
    for headers_sent in [false, true] {
        let mut f = Fixture::new(headers_sent).await;
        f.budget(Duration::from_secs(2));
        let page = f.page.clone();
        let mut work = Box::pin(page.goto("https://ipc-boundary.test/hold-navigation"));
        f.reached(Notice::Navigation, &mut work).await;
        let error = tokio::time::timeout(Duration::from_secs(10), work)
            .await
            .unwrap()
            .unwrap_err();
        ipc_deadline(&error, "goto");
        assert!(f.cancelled.load(Ordering::SeqCst) > 0);
        f.reaped();
        f.closed_without_replay().await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn real_worker_action_deadline_preserves_single_external_mutation_and_reaps() {
    let mut f = Fixture::new(false).await;
    let target = f
        .page
        .resolve_selector("#mutate", SemanticAction::Click)
        .await
        .unwrap();
    f.budget(Duration::from_secs(2));
    let page = f.page.clone();
    let mut work = Box::pin(page.execute(NativeAction {
        target,
        action: SemanticAction::Click,
        value: None,
    }));
    f.reached(Notice::Mutation, &mut work).await;
    let error = tokio::time::timeout(Duration::from_secs(10), work)
        .await
        .unwrap()
        .unwrap_err();
    ipc_deadline(&error, "execute");
    assert_eq!(f.mutations.load(Ordering::SeqCst), 1);
    f.reaped();
    f.closed_without_replay().await;
    assert_eq!(f.mutations.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn real_worker_explicit_close_cancels_inflight_navigation_and_consumes_wait() {
    let mut f = Fixture::new(true).await;
    let page = f.page.clone();
    let mut work = Box::pin(page.goto("https://ipc-boundary.test/hold-navigation"));
    f.reached(Notice::Navigation, &mut work).await;
    tokio::time::timeout(Duration::from_secs(10), f.page.close())
        .await
        .unwrap()
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(10), work)
            .await
            .unwrap()
            .is_err()
    );
    assert!(f.cancelled.load(Ordering::SeqCst) > 0);
    f.reaped();
    f.closed_without_replay().await;
}

#[tokio::test(flavor = "current_thread")]
async fn real_worker_close_waits_for_cancel_ack_then_succeeds_without_replay() {
    let mut f = Fixture::new(false).await;
    let id = f.hold_fetch_for_close().await;
    let page = f.page.clone();
    let mut close = Box::pin(page.close_ordered());
    f.reached(Notice::CancelEntered(id), &mut close).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut close)
            .await
            .is_err()
    );
    assert!(f.held.lock().unwrap().contains_key(&id));
    f.cancel_gate.release();
    tokio::time::timeout(Duration::from_secs(10), close)
        .await
        .unwrap()
        .unwrap();
    assert!(!f.held.lock().unwrap().contains_key(&id));
    f.reaped();
    f.closed_without_replay().await;
    assert_eq!(f.mutations.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn real_worker_close_ack_deadline_keeps_cleanup_owned_until_cancel_gate_releases() {
    let mut f = Fixture::new(false).await;
    let id = f.hold_fetch_for_close().await;
    f.budget(Duration::from_millis(500));
    let page = f.page.clone();
    let mut close = Box::pin(page.close_ordered());
    f.reached(Notice::CancelEntered(id), &mut close).await;
    assert_eq!(*f.cancel_gate.entered.lock().unwrap(), Some(id));
    let pending = tokio::time::timeout(Duration::from_millis(750), &mut close).await;
    assert!(
        pending.is_err(),
        "close must retain cleanup custody after IPC deadline until broker cancellation settles: {pending:?}"
    );
    f.cancel_gate.release();
    let error = tokio::time::timeout(Duration::from_secs(10), close)
        .await
        .unwrap()
        .unwrap_err();
    ipc_deadline(&error, "close");
    f.reaped();
    f.closed_without_replay().await;
}

#[tokio::test(flavor = "current_thread")]
async fn real_worker_explicit_action_cancellation_preserves_mutation_once_and_reaps() {
    let mut f = Fixture::new(false).await;
    let target = f
        .page
        .resolve_selector("#mutate", SemanticAction::Click)
        .await
        .unwrap();
    let page = f.page.clone();
    let mut action = Box::pin(page.execute(NativeAction {
        target,
        action: SemanticAction::Click,
        value: None,
    }));
    f.reached(Notice::Mutation, &mut action).await;
    // Explicit host close is the supported interrupt of executing worker script.
    // This acts only after the external mutation proves native action admission.
    tokio::time::timeout(Duration::from_secs(10), f.page.close())
        .await
        .unwrap()
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(10), action)
            .await
            .unwrap()
            .is_err()
    );
    assert_eq!(f.mutations.load(Ordering::SeqCst), 1);
    f.reaped();
    f.closed_without_replay().await;
    assert_eq!(f.mutations.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn real_worker_cancelled_close_caller_retains_cleanup_until_explicit_owner_retirement() {
    let mut f = Fixture::new(false).await;
    let id = f.hold_fetch_for_close().await;
    let page = f.page.clone();
    let task = tokio::spawn(async move { page.close_ordered().await });
    let mut close = Box::pin(task);
    f.reached(Notice::CancelEntered(id), &mut close).await;
    assert_eq!(*f.cancel_gate.entered.lock().unwrap(), Some(id));
    close.as_ref().get_ref().abort();
    assert!(close.await.unwrap_err().is_cancelled());
    // Aborting an async waiter does not cancel spawn_blocking IPC. Explicit
    // context retirement remains the owner and must wait for actual cleanup.
    let context = f._context.clone();
    let mut retirement = Box::pin(context.retire_workers());
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut retirement)
            .await
            .is_err(),
        "cancelled close caller must not turn unsettled native cleanup into success"
    );
    f.cancel_gate.release();
    tokio::time::timeout(Duration::from_secs(10), retirement)
        .await
        .unwrap()
        .unwrap();
    f.reaped();
    f.closed_without_replay().await;
    assert_eq!(f.mutations.load(Ordering::SeqCst), 0);
}
