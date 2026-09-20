use super::*;
use crate::network::{NetworkBroker, NetworkError, NetworkRequest, NetworkResponse};

#[derive(Default)]
struct Gate {
    released: Mutex<bool>,
    changed: std::sync::Condvar,
    entered: AtomicBool,
    hold_child_until_ack: AtomicBool,
    child_ready: Mutex<bool>,
    child_changed: std::sync::Condvar,
}
struct Broker(Arc<Gate>);
impl NetworkBroker for Broker {
    fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        let path = Url::parse(&request.url).unwrap().path().to_owned();
        if path == "/release" {
            *self.0.released.lock().unwrap() = true;
            self.0.changed.notify_all();
        }
        if path == "/entered" {
            self.0.entered.store(true, Ordering::Release);
        }
        if path == "/own" {
            self.0.entered.store(true, Ordering::Release);
        }
        let body = if path == "/child" || path == "/own" {
            let (released, _) = self
                .0
                .changed
                .wait_timeout_while(
                    self.0.released.lock().unwrap(),
                    Duration::from_secs(20),
                    |released| !*released,
                )
                .unwrap();
            if !*released {
                return Err(NetworkError::TimedOut {
                    timeout: Duration::from_secs(20),
                });
            }
            drop(released);
            if path == "/child" && self.0.hold_child_until_ack.load(Ordering::Acquire) {
                let (ready, _) = self
                    .0
                    .child_changed
                    .wait_timeout_while(
                        self.0.child_ready.lock().unwrap(),
                        Duration::from_secs(20),
                        |ready| !*ready,
                    )
                    .unwrap();
                if !*ready {
                    return Err(NetworkError::TimedOut {
                        timeout: Duration::from_secs(20),
                    });
                }
            }
            if path == "/own" {
                b"<!doctype html><body id=own><script>waitDone=false;window.open('/child')</script></body>".to_vec()
            } else {
                b"<!doctype html><body id=child><script>opener.top.waitDone=true</script></body>"
                    .to_vec()
            }
        } else {
            b"<!doctype html><body id=ready>ready</body>".to_vec()
        };
        Ok(NetworkResponse {
            response_policy: None,
            timing: None,
            request_id: request.request_id,
            status: 200,
            headers: vec![("content-type".into(), "text/html".into())],
            body,
        })
    }
}
async fn entered(gate: &Gate) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while !gate.entered.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("native wait predicate entered");
}
fn fixture() -> (Browser, BrowserContext, Arc<Gate>) {
    let browser = Browser::native_test_with_config(BrowserConfig {
        command_timeout: Duration::from_secs(20),
        ..Default::default()
    });
    let gate = Arc::new(Gate::default());
    let context = browser.new_native_test_context_with_broker(
        Default::default(),
        SharedNetworkBroker::new(Broker(gate.clone())),
    );
    (browser, context, gate)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn readiness_waits_advance_related_navigation_for_top_and_selected_frame() {
    use crate::host::webdriver_script::{FrameCommand, FrameResponse};
    for scoped in [false, true] {
        for explicit in [false, true] {
            let (_browser, context, _gate) = fixture();
            let page = context.new_page();
            page.load_html("https://wait-fairness.test/source", "<!doctype html><body>source<iframe srcdoc='<body>selected</body>'></iframe></body>").await.unwrap();
            let selected = if scoped {
                let FrameResponse::Inventory(realms) = page
                    .webdriver_frames(FrameCommand::Inventory)
                    .await
                    .unwrap()
                else {
                    panic!("inventory");
                };
                page.with_frame_scope(Some(
                    realms
                        .into_iter()
                        .find(|entry| entry.live && entry.parent_realm.is_some())
                        .unwrap(),
                ))
            } else {
                page.clone()
            };
            selected.evaluate("top.waitDone=false;globalThis.sent=false;globalThis.popup=window.open('/child');true").await.unwrap();
            // Only this real readiness wait releases the broker response. The
            // child therefore cannot finish before the source command starts.
            let predicate = WaitPredicate::JavaScript(
                "(()=>{if(!sent){sent=true;fetch('/release')}return top.waitDone})()".into(),
            );
            if explicit {
                selected
                    .wait_for_timeout(predicate, Some(Duration::from_secs(20)))
                    .await
                    .unwrap();
            } else {
                selected.wait_for(predicate).await.unwrap();
            }
            assert_eq!(
                page.evaluate(
                    "String(waitDone && document.body.firstChild.textContent==='source')"
                )
                .await
                .unwrap(),
                "true"
            );
            let child = context
                .wait_for_page(
                    PagePredicate::Opener {
                        page_id: page.page_id(),
                    },
                    Duration::from_secs(20),
                )
                .await
                .unwrap();
            assert_eq!(child.evaluate("document.body.id").await.unwrap(), "child");
            child.close().await.unwrap();
            page.close().await.unwrap();
        }
    }
}

// Dropping a direct actor response receiver cancels its retained wait. A
// remote spawn_blocking request is instead interrupted by explicit close below.
#[cfg(feature = "v8-host")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_wait_preserves_same_page_fifo_and_releases_barrier() {
    let (_browser, context, gate) = fixture();
    let page = context.new_page();
    page.load_html(
        "https://wait-fairness.test/source",
        "<!doctype html><body>source</body>",
    )
    .await
    .unwrap();
    let waiting = page.clone();
    let waiter = tokio::spawn(async move {
        waiting
            .wait_for_timeout(
                WaitPredicate::JavaScript(
                    "(()=>{if(!globalThis.entered){entered=true;fetch('/entered')}return false})()"
                        .into(),
                ),
                None,
            )
            .await
    });
    entered(&gate).await;
    let following = page.clone();
    let next = tokio::spawn(async move {
        following
            .evaluate("globalThis.following=true;'following'")
            .await
    });
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(
        !next.is_finished(),
        "later command must stay behind the wait"
    );
    waiter.abort();
    let _ = waiter.await;
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(20), next)
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        "following"
    );
    page.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_close_cancels_an_indefinite_wait_before_queued_close() {
    let (_browser, context, gate) = fixture();
    let page = context.new_page();
    page.load_html(
        "https://wait-fairness.test/source",
        "<!doctype html><body>source</body>",
    )
    .await
    .unwrap();
    let waiting = page.clone();
    let waiter = tokio::spawn(async move {
        waiting
            .wait_for_timeout(
                WaitPredicate::JavaScript(
                    "(()=>{if(!globalThis.entered){entered=true;fetch('/entered')}return false})()"
                        .into(),
                ),
                None,
            )
            .await
    });
    entered(&gate).await;
    tokio::time::timeout(Duration::from_secs(20), page.close())
        .await
        .unwrap()
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(20), waiter)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zero_deadline_checks_once_and_scoped_replacement_never_rebinds_wait() {
    use crate::host::webdriver_script::{FrameCommand, FrameResponse};
    let (_browser, context, _gate) = fixture();
    let page = context.new_page();
    page.load_html(
        "https://wait-fairness.test/source",
        "<!doctype html><body>source<iframe srcdoc='<body>selected</body>'></iframe></body>",
    )
    .await
    .unwrap();
    assert!(
        page.wait_for_timeout(
            WaitPredicate::JavaScript("(globalThis.probes=(globalThis.probes||0)+1,false)".into()),
            Some(Duration::ZERO)
        )
        .await
        .is_err()
    );
    assert_eq!(page.evaluate("String(probes)").await.unwrap(), "1");
    let FrameResponse::Inventory(realms) = page
        .webdriver_frames(FrameCommand::Inventory)
        .await
        .unwrap()
    else {
        panic!("inventory");
    };
    let frame = page.with_frame_scope(Some(
        realms
            .into_iter()
            .find(|entry| entry.live && entry.parent_realm.is_some())
            .unwrap(),
    ));
    assert!(frame.wait_for_timeout(WaitPredicate::JavaScript("(()=>{if(!globalThis.replacing){replacing=true;frameElement.srcdoc='<body id=replaced>replacement</body>'}return false})()".into()), Some(Duration::from_secs(20))).await.is_err());
    assert_eq!(page.evaluate("String(document.body.firstChild.textContent==='source' && frames[0].document.body.id==='replaced')").await.unwrap(), "true");
    page.close().await.unwrap();
}

// Direct actor requests can coexist while navigation replies remain pending.
// A remote WorkerSlot serializes requests under its client mutex.
#[cfg(feature = "v8-host")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retained_wait_finishes_own_navigation_reply_before_related_child_response() {
    let (_browser, context, gate) = fixture();
    gate.hold_child_until_ack.store(true, Ordering::Release);
    let page = context.new_page();
    page.load_html(
        "https://wait-fairness.test/source",
        "<!doctype html><body>source</body>",
    )
    .await
    .unwrap();
    let navigating = page.clone();
    let navigation =
        tokio::spawn(async move { navigating.goto("https://wait-fairness.test/own").await });
    entered(&gate).await;
    let waiting = page.clone();
    let waiter = tokio::spawn(async move {
        waiting.wait_for_timeout(WaitPredicate::JavaScript("(()=>{if(!globalThis.releasedOnce){releasedOnce=true;fetch('/release')}return globalThis.waitDone===true})()".into()), Some(Duration::from_secs(20))).await
    });
    // The native navigation reply must advance while the source wait remains
    // pending. The related child cannot answer until this acknowledgement.
    tokio::time::timeout(Duration::from_secs(20), navigation)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(!waiter.is_finished());
    *gate.child_ready.lock().unwrap() = true;
    gate.child_changed.notify_all();
    tokio::time::timeout(Duration::from_secs(20), waiter)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(page.evaluate("document.body.id").await.unwrap(), "own");
    let child = context
        .wait_for_page(
            PagePredicate::Opener {
                page_id: page.page_id(),
            },
            Duration::from_secs(20),
        )
        .await
        .unwrap();
    child.close().await.unwrap();
    page.close().await.unwrap();
}
