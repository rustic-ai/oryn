use super::*;
use crate::network::{NetworkBroker, NetworkError, NetworkRequest, NetworkResponse};

struct InitialPopupBroker(Arc<Mutex<Vec<NetworkRequest>>>);
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_initial_blank_popup_keeps_exposed_document_and_synchronous_edits() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let browser = Browser::native_test_with_config(Default::default());
    let context = browser.new_native_test_context_with_broker(
        Default::default(), SharedNetworkBroker::new(InitialPopupBroker(requests.clone())),
    );
    let opener = context.new_page();
    opener.load_html("https://popup.test/source", "<!doctype html><body>source</body>").await.unwrap();
    opener.evaluate("globalThis.popup=open('about:blank');globalThis.initial=popup.document;popup.document.body.innerHTML='<p id=written>saved</p>';popup.marker=73;true").await.unwrap();
    let child = context.wait_for_page(PagePredicate::Opener { page_id: opener.page_id() }, Duration::from_secs(2)).await.unwrap();
    assert_eq!(child.evaluate("String(document.getElementById('written').textContent==='saved'&&marker===73)").await.unwrap(), "true");
    assert_eq!(opener.evaluate("String(popup.document===initial&&popup.opener===window)").await.unwrap(), "true");
    assert!(requests.lock().unwrap().is_empty());
    assert_eq!(context.queued_navigation_status(child.page_id()).unwrap().active, 0);
    child.close().await.unwrap();
    opener.close().await.unwrap();
}
impl NetworkBroker for InitialPopupBroker {
    fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        let request_id = request.request_id;
        self.0.lock().unwrap().push(request);
        Ok(NetworkResponse {
            response_policy: None,
            timing: None,
            request_id,
            status: 200,
            headers: vec![("content-type".into(), "text/html".into())],
            body: b"<!doctype html><body id=loaded><script>opener.initialPopupLoaded=true</script></body>".to_vec(),
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn initial_popup_navigation_executes_without_a_child_command_and_replaces_blank() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let browser = Browser::native_test_with_config(Default::default());
    let context = browser.new_native_test_context_with_broker(
        Default::default(),
        SharedNetworkBroker::new(InitialPopupBroker(requests.clone())),
    );
    let opener = context.new_page();
    let source = "https://popup.test/source?query=1";
    let destination = "https://popup.test/loaded";
    opener
        .load_html(source, "<!doctype html><body>opener</body>")
        .await
        .unwrap();
    assert_eq!(opener.evaluate(
        "globalThis.initialPopupLoaded=false;globalThis.popup=window.open('/loaded');globalThis.initialPopupDocument=popup.document;String(popup!==null&&popup.opener===window&&popup.location.href==='about:blank')"
    ).await.unwrap(), "true");

    // No child PageHandle command or source polling can bootstrap this load.
    // The related actor must consume its already admitted initial navigation.
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if !requests.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("initial child navigation must reach the native broker autonomously");
    {
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.url, destination);
        assert_eq!(request.method, "GET");
        assert_eq!(request.security.initiator_url.as_deref(), Some(source));
        assert_eq!(request.security.top_level_url.as_deref(), Some(source));
    }
    // Keep each source observation bounded to one command. A blocking source
    // WaitFor would itself occupy the shared actor while the child needs a turn.
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if opener.evaluate("String(initialPopupLoaded)").await.unwrap() == "true" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("admitted child document script must execute");
    assert_eq!(opener.evaluate(
        "String(popup.document!==initialPopupDocument&&popup.document.getElementById('loaded')!==null&&popup.history.length===1&&popup.opener===window&&document.body.textContent==='opener')"
    ).await.unwrap(), "true");
    let popup = context
        .wait_for_page(
            PagePredicate::Opener {
                page_id: opener.page_id(),
            },
            Duration::from_secs(2),
        )
        .await
        .unwrap();
    let pending = context.queued_navigation_status(popup.page_id()).unwrap();
    assert_eq!(pending.active, 0);
    assert!(pending.outcomes.is_empty());
    popup.close().await.unwrap();
    assert_eq!(
        opener.evaluate("String(popup.closed)").await.unwrap(),
        "true"
    );
    opener.close().await.unwrap();
}
