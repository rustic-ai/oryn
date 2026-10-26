//! Production-worker popup must navigate before its first explicit command.
use super::*;
use std::io::{Read, Write};

struct ExecutorFixture {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    server: Option<std::thread::JoinHandle<()>>,
}
impl ExecutorFixture {
    fn finish(mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        self.server.take().unwrap().join().unwrap();
    }
}
impl Drop for ExecutorFixture {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        if let Some(server) = self.server.take() {
            let _ = server.join();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_popup_navigates_before_first_destination_command() {
    assert!(
        std::env::var_os("ORYN_PAGE_WORKER").is_some(),
        "production-path regression requires an explicitly selected signed worker"
    );
    for host in ["source.test", "foreign.test"] {
        // Unit-test builds default to in-process; explicitly select production
        // placement rather than accidentally exercising the direct root path.
        let mut browser = Browser::new();
        browser.worker_process = true;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let destination = format!("http://{host}:{port}/executor");
        let (requested, mut requests) = tokio::sync::mpsc::unbounded_channel();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let server_stop = stop.clone();
        let server = std::thread::spawn(move || {
            while !server_stop.load(std::sync::atomic::Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        stream
                            .set_write_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        let mut header = Vec::new();
                        let mut byte = [0];
                        while header.len() < 16384 && !header.ends_with(b"\r\n\r\n") {
                            if stream.read(&mut byte).unwrap_or(0) == 0 {
                                break;
                            }
                            header.push(byte[0]);
                        }
                        if header.starts_with(b"GET /executor ") {
                            let _ = requested.send(());
                        }
                        let body = "<!doctype html><p>executor</p><script>globalThis.executorStarted=true</script>";
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = stream.write_all(response.as_bytes());
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("popup fixture accept: {error}"),
                }
            }
        });
        let fixture = ExecutorFixture {
            stop,
            server: Some(server),
        };
        let routes =
            crate::network::EndpointRoutes::new(["source.test", "foreign.test"].map(|host| {
                (
                    "http".into(),
                    host.into(),
                    port,
                    "127.0.0.1".parse().unwrap(),
                )
            }))
            .unwrap();
        browser = browser.with_fixture_network(Vec::new(), routes).unwrap();
        // Unlike the custom-broker constructor, this is the same isolated
        // auxiliary production placement selected by WebDriver sessions.
        let context =
            browser.new_audited_context_with_isolated_auxiliaries(ContextOptions::loopback_test());
        assert!(context.inner.worker_process);
        let source = context.new_page();
        assert!(source.worker.is_some());
        source
            .load_html(
                &format!("http://source.test:{port}/start"),
                "<!doctype html><p>source</p>",
            )
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
        if !matches!(reached, Ok(Some(()))) {
            // Read only after the original no-command arrival deadline failed;
            // this diagnostic cannot turn that failed observation into a pass.
            for entry in context.pages() {
                if entry.page.page_id != source.page_id() {
                    if let Some(handle) = context.inner.group.handle(entry.page.page_id) {
                        eprintln!("failed initial popup trace: {:?}", handle.trace().await);
                    }
                }
            }
        }
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
        fixture.finish();
        assert_eq!(observed.unwrap(), "true");
    }
}
