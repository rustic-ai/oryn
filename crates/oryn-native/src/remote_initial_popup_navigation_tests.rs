//! Production-worker popup must navigate before its first explicit command.
use super::*;
use crate::network::{NetworkBroker, NetworkError, NetworkRequest, NetworkResponse};

struct Executor(tokio::sync::mpsc::UnboundedSender<()>);
impl NetworkBroker for Executor {
    fn send(&self, request: NetworkRequest) -> Result<NetworkResponse, NetworkError> {
        if request.url.ends_with("/executor") {
            let _ = self.0.send(());
        }
        Ok(NetworkResponse {
            response_policy: None,
            timing: None,
            request_id: request.request_id,
            status: 200,
            headers: vec![("content-type".into(), "text/html".into())],
            body: b"<!doctype html><p>executor</p><script>globalThis.executorStarted=true</script>"
                .to_vec(),
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_popup_navigates_before_first_destination_command() {
    assert!(
        std::env::var_os("ORYN_PAGE_WORKER").is_some(),
        "production-path regression requires an explicitly selected signed worker"
    );
    for destination in [
        "https://source.test/executor",
        "https://foreign.test/executor",
    ] {
        // Unit-test builds default to in-process; explicitly select production
        // placement rather than accidentally exercising the direct root path.
        let mut browser = Browser::new();
        browser.worker_process = true;
        let (requested, mut requests) = tokio::sync::mpsc::unbounded_channel();
        let context = browser.new_context_with_broker(
            Default::default(),
            SharedNetworkBroker::new(Executor(requested)),
        );
        let source = context.new_page();
        source
            .load_html("https://source.test/start", "<!doctype html><p>source</p>")
            .await
            .unwrap();
        assert_eq!(
            source
                .evaluate(&format!(
                    "globalThis.popup=window.open({destination:?});String(popup!==null)"
                ))
                .await
                .unwrap(),
            "true"
        );
        // No destination command may bootstrap the controller under test.
        let reached = tokio::time::timeout(Duration::from_secs(5), requests.recv()).await;
        let result = if matches!(reached, Ok(Some(()))) {
            context
                .wait_for_page(
                    PagePredicate::Opener {
                        page_id: source.page_id(),
                    },
                    Duration::from_secs(5),
                )
                .await
        } else {
            Err(RuntimeError::PageWaitTimeout)
        };
        let observed = match result {
            Ok(popup) => match popup.wait_for_timeout(
                WaitPredicate::UrlContains(destination.into()), Some(Duration::from_secs(5))
            ).await {
                Ok(_) => match popup.wait_for_timeout(WaitPredicate::Lifecycle(LifecycleState::Load),
                    Some(Duration::from_secs(5))).await {
                    Ok(_) => popup.evaluate(
                        "String(executorStarted===true&&document.querySelector('p').textContent==='executor')"
                    ).await,
                    Err(error) => Err(error),
                },
                Err(error) => Err(error),
            },
            Err(error) => Err(error),
        };
        // Retain teardown custody even when the initial navigation never arrives.
        for page in context.pages() {
            context.close_page(page.page.page_id).await.unwrap();
        }
        context.retire_workers().await.unwrap();
        assert_eq!(observed.unwrap(), "true");
    }
}
