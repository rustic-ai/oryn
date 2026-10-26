//! Initial Window.open URL must reach its own adopted root without another goto.
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
async fn initial_same_and_cross_origin_popup_navigation_survives_actor_adoption() {
    for destination in [
        "https://source.test/executor",
        "https://foreign.test/executor",
    ] {
        let (requested, mut requests) = tokio::sync::mpsc::unbounded_channel();
        let context = Browser::in_process_probe().new_context_with_broker(
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
        tokio::time::timeout(Duration::from_secs(5), requests.recv())
            .await
            .expect("initial admitted navigation never reached its broker")
            .unwrap();
        let popup = context
            .wait_for_page(
                PagePredicate::Opener {
                    page_id: source.page_id(),
                },
                Duration::from_secs(5),
            )
            .await
            .expect("initial admitted navigation never reached its popup");
        popup
            .wait_for_timeout(
                WaitPredicate::UrlContains(destination.into()),
                Some(Duration::from_secs(5)),
            )
            .await
            .unwrap();
        popup
            .wait_for_timeout(
                WaitPredicate::Lifecycle(LifecycleState::Load),
                Some(Duration::from_secs(5)),
            )
            .await
            .unwrap();
        assert_eq!(popup.evaluate("String(executorStarted===true&&document.querySelector('p').textContent==='executor')").await.unwrap(), "true");
        assert_eq!(
            source.evaluate("document.body.textContent").await.unwrap(),
            "source"
        );
        popup.close().await.unwrap();
        source.close().await.unwrap();
        context.retire_workers().await.unwrap();
    }
}
