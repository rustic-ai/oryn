//! Production-worker popup must navigate before its first explicit command.
use super::*;
use std::io::{Read, Write};
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn production_worker_redirect_csp_event_reaches_worker_listener() {
    assert!(std::env::var_os("ORYN_PAGE_WORKER").is_some());
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
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
                    let (status, extra, body) = if header.starts_with(b"GET /worker.js ") {
                        (200, "Content-Type: text/javascript\r\nContent-Security-Policy: connect-src 'self'; report-uri /report\r\n".to_owned(),
                            "const original=new URL('/redirect?case=original',self.location.href).href;let blocked=null,rejected=false;function publish(){if(blocked!==null&&rejected)postMessage(blocked===original)}addEventListener('securitypolicyviolation',e=>{if(e.blockedURI===original){blocked=e.blockedURI;publish()}});(async()=>{const foreign=self.location.origin.replace('source.test','foreign.test');await fetch(foreign+'/first').catch(()=>{});await fetch(foreign+'/second').catch(()=>{});fetch(original).then(()=>postMessage('unexpected success')).catch(e=>{rejected=e instanceof TypeError;publish()})})();".to_owned())
                    } else if header.starts_with(b"GET /redirect?") {
                        (
                            307,
                            format!("Location: http://foreign.test:{port}/denied\r\n"),
                            String::new(),
                        )
                    } else {
                        (
                            200,
                            "Content-Type: text/plain\r\n".to_owned(),
                            String::new(),
                        )
                    };
                    let response = format!(
                        "HTTP/1.1 {status} OK\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes());
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(2))
                }
                Err(error) => panic!("worker CSP fixture: {error}"),
            }
        }
    });
    let fixture = ExecutorFixture {
        stop,
        server: Some(server),
    };
    let routes = crate::network::EndpointRoutes::new(["source.test", "foreign.test"].map(|host| {
        (
            "http".into(),
            host.into(),
            port,
            "127.0.0.1".parse().unwrap(),
        )
    }))
    .unwrap();
    let mut browser = Browser::new();
    browser.worker_process = true;
    let context = browser
        .with_fixture_network(Vec::new(), routes)
        .unwrap()
        .new_audited_context_with_isolated_auxiliaries(ContextOptions::loopback_test());
    let page = context.new_page();
    page.load_html(
        &format!("http://source.test:{port}/start"),
        "<!doctype html><p>source</p>",
    )
    .await
    .unwrap();
    page.evaluate("globalThis.workerResult=null;globalThis.worker=new Worker('/worker.js');worker.onmessage=e=>globalThis.workerResult=e.data;worker.onerror=()=>globalThis.workerResult='worker error';true").await.unwrap();
    let wait = page
        .wait_for_timeout(
            WaitPredicate::JavaScript("workerResult!==null".into()),
            Some(Duration::from_secs(5)),
        )
        .await;
    let result = page.evaluate("String(workerResult)").await;
    for entry in context.pages() {
        context.close_page(entry.page.page_id).await.unwrap();
    }
    context.retire_workers().await.unwrap();
    fixture.finish();
    assert!(
        wait.is_ok(),
        "worker listener completion: {wait:?}; result: {result:?}"
    );
    assert_eq!(result.unwrap(), "true");
}

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
        let destination = format!("http://{host}:{port}/executor?pipe=&uuid=popup-probe");
        let (requested, mut requests) = tokio::sync::mpsc::unbounded_channel();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let server_stop = stop.clone();
        let server = std::thread::spawn(move || {
            let ping_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let mut connections = Vec::new();
            while !server_stop.load(std::sync::atomic::Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let stop = server_stop.clone();
                        let requested = requested.clone();
                        let ping_count = ping_count.clone();
                        connections.push(std::thread::spawn(move || {
                        stream
                            .set_read_timeout(Some(Duration::from_millis(100)))
                            .unwrap();
                        stream
                            .set_write_timeout(Some(Duration::from_secs(2)))
                            .unwrap();
                        while !stop.load(std::sync::atomic::Ordering::Acquire) {
                        let mut header = Vec::new();
                        let mut byte = [0];
                        while header.len() < 16384 && !header.ends_with(b"\r\n\r\n") {
                            match stream.read(&mut byte) {
                                Ok(0) => return,
                                Ok(_) => header.push(byte[0]),
                                Err(error) if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                                    if stop.load(std::sync::atomic::Ordering::Acquire) { return; }
                                }
                                Err(_) => return,
                            }
                        }
                        if !header.ends_with(b"\r\n\r\n") { return; }
                        let count = if header.starts_with(b"POST /ping?uuid=popup-probe ") {
                            ping_count.fetch_add(1, std::sync::atomic::Ordering::AcqRel) + 1
                        } else { 0 };
                        if header.starts_with(b"GET /executor?") {
                            let _ = requested.send("executor");
                        } else if header.starts_with(b"POST /ping?uuid=popup-probe ") {
                            let length = String::from_utf8_lossy(&header).lines()
                                .find_map(|line| line.to_ascii_lowercase().strip_prefix("content-length:")
                                    .and_then(|value| value.trim().parse::<usize>().ok())).unwrap_or(0);
                            let mut body = vec![0; length.min(16384)];
                            if stream.read_exact(&mut body).is_ok() && body == b"popup-payload" && count == 2 {
                                let _ = requested.send("ping");
                            }
                        } else if header.starts_with(b"GET /dynamic.js ") {
                            let _ = requested.send("script");
                        }
                        let body = if header.starts_with(b"POST /ping?uuid=popup-probe ") && count == 1 {
                            "not ready"
                        } else if header.starts_with(b"POST /ping?uuid=popup-probe ") {
                            "globalThis.fetchResult='pong';const importScript=url=>{const s=document.createElement('script');s.src=url;const loaded=new Promise(resolve=>s.onload=()=>{globalThis.scriptLoaded=true;resolve()});document.body.appendChild(s);return loaded};await importScript('/dynamic.js');document.body.appendChild(document.createElement('iframe'));"
                        } else if header.starts_with(b"GET /dynamic.js ") {
                            "globalThis.dynamicLoaded=true"
                        } else {
                            "<!doctype html><p>executor</p><script>globalThis.executorStarted=true;setTimeout(async()=>{try{let v;do{v=await(await fetch('/ping?uuid=popup-probe',{method:'POST',body:'popup-payload'})).text();if(v==='not ready')await new Promise(resolve=>setTimeout(resolve,20))}while(v==='not ready');eval('(async()=>{'+v+'})()')}catch(e){globalThis.fetchResult=e.name}},20)</script>"
                        };
                        let media = if header.starts_with(b"GET /dynamic.js ") { "text/javascript" } else { "text/html" };
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: {media}\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{body}",
                            body.len()
                        );
                        if stream.write_all(response.as_bytes()).is_err() { return; }
                        }
                        }));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("popup fixture accept: {error}"),
                }
            }
            for connection in connections {
                connection.join().unwrap();
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
        let ping = if matches!(reached, Ok(Some("executor"))) {
            matches!(
                tokio::time::timeout(Duration::from_secs(5), requests.recv()).await,
                Ok(Some("ping"))
            )
        } else {
            false
        };
        let script = if ping {
            matches!(
                tokio::time::timeout(Duration::from_secs(5), requests.recv()).await,
                Ok(Some("script"))
            )
        } else {
            false
        };
        if !matches!(reached, Ok(Some("executor"))) {
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
        let result = if matches!(reached, Ok(Some("executor"))) {
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
            Ok(popup) => {
                match popup
                    .wait_for_timeout(
                        WaitPredicate::UrlContains(destination.into()),
                        Some(Duration::from_secs(5)),
                    )
                    .await
                {
                    Ok(_) => {
                        match popup
                            .wait_for_timeout(
                                WaitPredicate::Lifecycle(LifecycleState::Load),
                                Some(Duration::from_secs(5)),
                            )
                            .await
                        {
                            Ok(_) => {
                                if !ping {
                                    eprintln!("popup fetch result: {:?}", popup.evaluate("typeof fetchResult==='undefined'?'pending':fetchResult").await);
                                    Err(RuntimeError::PageWaitTimeout)
                                } else {
                                    if !script {
                                        eprintln!("popup dynamic script state: {:?}", popup.evaluate("JSON.stringify({fetch:typeof fetchResult==='undefined'?'pending':fetchResult,scripts:document.querySelectorAll('script').length,loaded:globalThis.scriptLoaded===true})").await);
                                        Err(RuntimeError::PageWaitTimeout)
                                    } else {
                                        match popup.wait_for_timeout(WaitPredicate::JavaScript(
                                    "globalThis.dynamicLoaded===true&&globalThis.scriptLoaded===true&&frames.length===1".into()
                                ), Some(Duration::from_secs(5))).await {
                                    Ok(_) => popup.evaluate(
                                        "String(executorStarted===true&&fetchResult==='pong'&&new URLSearchParams(location.search).get('uuid')==='popup-probe'&&document.querySelector('p').textContent==='executor')"
                                    ).await,
                                    Err(error) => Err(error),
                                }
                                    }
                                }
                            }
                            Err(error) => Err(error),
                        }
                    }
                    Err(error) => Err(error),
                }
            }
            Err(error) => Err(error),
        };
        if observed.is_err() {
            eprintln!("popup descriptors after failure: {:?}", context.pages());
            for entry in context.pages() {
                if entry.page.page_id != source.page_id() {
                    if let Some(handle) = context.inner.group.handle(entry.page.page_id) {
                        eprintln!("failed popup completion trace: {:?}", handle.trace().await);
                    }
                }
            }
        }
        // Retain teardown custody even when the initial navigation never arrives.
        for page in context.pages() {
            context.close_page(page.page.page_id).await.unwrap();
        }
        context.retire_workers().await.unwrap();
        fixture.finish();
        assert_eq!(observed.unwrap(), "true");
    }
}
