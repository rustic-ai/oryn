use chromiumoxide::browser::{Browser, BrowserConfig};
use futures::StreamExt;
use lscope_core::backend::Backend;
use lscope_core::protocol::{ScanRequest, ScannerRequest};
use lscope_r::backend::RemoteBackend;
use serial_test::serial;
use std::fs;
use std::time::Duration;
use tempfile::tempdir;
use tokio::time::timeout;

#[tokio::test]
#[serial]
async fn test_remote_extension_e2e() {
    tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::INFO)
        .try_init()
        .ok();

    println!("STEP 1: Selecting dynamic port...");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    println!("Selected port for test: {}", port);

    println!("STEP 2: Patching extension...");
    let root = std::env::current_dir().unwrap();
    let src_extension_path = root.join("../../extension");

    let tmp_dir = tempdir().unwrap();
    let ext_tmp_path = tmp_dir.path();

    for entry in fs::read_dir(src_extension_path).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_file() {
            let dest = ext_tmp_path.join(path.file_name().unwrap());
            fs::copy(&path, &dest).unwrap();
        }
    }

    let bg_path = ext_tmp_path.join("background.js");
    let bg_content = fs::read_to_string(&bg_path).unwrap();
    let patched_content = bg_content.replace(":9001", &format!(":{}", port));
    fs::write(&bg_path, patched_content.clone()).unwrap();
    println!(
        "Patched background.js: {}",
        patched_content.lines().next().unwrap()
    );

    let extension_path_str = ext_tmp_path.to_str().expect("Valid path");

    println!("STEP 3: Launching browser with extension...");
    // Try to use headless=new which supports extensions better
    let (mut browser, mut handler) = Browser::launch(
        BrowserConfig::builder()
            .chrome_executable("/usr/lib64/chromium-browser/chromium-browser")
            .no_sandbox()
            .arg(format!(
                "--disable-extensions-except={}",
                extension_path_str
            ))
            .arg(format!("--load-extension={}", extension_path_str))
            .build()
            .unwrap(),
    )
    .await
    .unwrap();

    let _handle = tokio::spawn(async move {
        while let Some(h) = handler.next().await {
            if let Err(e) = h {
                eprintln!("Browser handler error: {}", e);
                break;
            }
        }
    });

    println!("STEP 4: Starting Remote Backend...");
    let mut backend = RemoteBackend::new(port);
    backend.launch().await.expect("Failed to launch backend");

    println!("STEP 5: Navigating to test harness...");
    let page = browser.new_page("about:blank").await.unwrap();
    // Use test harness which is already running
    let url = "http://localhost:3000/scenarios/01_static.html";
    if let Err(e) = page.goto(url).await {
        println!(
            "Warning: Failed to navigate to {}, falling back to data URL. Error: {}",
            url, e
        );
        let html = "<html><head><title>Remote Test</title></head><body><h1>Hello</h1><button id='target'>Action</button></body></html>";
        page.goto(format!("data:text/html,{}", html)).await.unwrap();
    }

    println!(
        "STEP 6: Waiting for extension connection to port {}...",
        port
    );
    // Extensions in headless might take a bit to wake up
    for i in 1..=15 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if backend.is_ready().await {
            // Check if we can send a "ping" or just hope
            // Actually, we don't have a way to check connection count in RemoteBackend easily
        }
        println!("  Waiting... {}s", i);
    }

    println!("STEP 7: Sending scan request...");
    let scan_req = ScannerRequest::Scan(ScanRequest {
        max_elements: None,
        monitor_changes: false,
        include_hidden: false,
        view_all: true,
    });

    let resp_result = timeout(Duration::from_secs(15), backend.execute_scanner(scan_req)).await;

    match resp_result {
        Ok(Ok(resp)) => {
            println!("SUCCESS: Received response from extension!");
            if let lscope_core::protocol::ScannerProtocolResponse::Ok { data, .. } = resp {
                match *data {
                    lscope_core::protocol::ScannerData::Scan(result) => {
                        println!("Result URL: {}", result.page.url);
                        println!("Result Title: {}", result.page.title);
                        assert!(!result.elements.is_empty());
                    }
                    _ => panic!("Expected Scan result, got: {:?}", data),
                }
            } else {
                panic!("Scanner returned error: {:?}", resp);
            }
        }
        Ok(Err(e)) => panic!("Backend error: {}", e),
        Err(_) => {
            let bg_log = fs::read_to_string(&bg_path).unwrap();
            println!("Patched background.js content:\n{}", bg_log);
            panic!(
                "Timeout waiting for extension response. Extension failed to connect to port {}.",
                port
            );
        }
    }

    println!("STEP 8: Cleanup...");
    backend.close().await.ok();
    browser.close().await.ok();
}
