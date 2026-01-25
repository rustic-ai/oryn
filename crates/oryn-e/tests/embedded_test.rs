//! Embedded backend integration tests
//!
//! These tests launch real COG browser instances via WPEWebDriver.
//! Tests run sequentially via `#[serial]` to avoid resource contention.

use oryn_e::backend::EmbeddedBackend;
use oryn_engine::backend::Backend;
use oryn_engine::protocol::ScannerAction;
use serial_test::serial;

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}

#[tokio::test]
#[serial]
#[ignore] // Requires weston/cog - run via ./scripts/run-tests.sh or in Docker
async fn test_embedded_lifecycle() {
    init_tracing();
    // Test basic lifecycle: launch -> navigate -> close
    // Use port 8081 to avoid conflicts with parallel tests
    let mut backend = EmbeddedBackend::new_headless_on_port(8081);

    // Launch (auto-starts WPEWebDriver + COG)
    backend.launch().await.expect("Failed to launch backend");
    println!("Backend launched successfully.");

    // Navigate
    let res = backend.navigate("https://example.com").await;
    assert!(res.is_ok(), "Navigation failed: {:?}", res.err());
    let nav_res = res.unwrap();
    println!("Navigated to: {}", nav_res.url);
    assert!(nav_res.url.contains("example.com"));

    // Close
    let close_res = backend.close().await;
    assert!(close_res.is_ok(), "Close failed: {:?}", close_res.err());
}

#[tokio::test]
#[serial]
#[ignore] // Requires weston/cog - run via ./scripts/run-tests.sh or in Docker
async fn test_embedded_features() {
    init_tracing();
    // Use port 8082 to avoid conflicts with parallel tests
    let mut backend = EmbeddedBackend::new_headless_on_port(8082);

    backend.launch().await.expect("Failed to launch backend");

    // Navigate to a real page first
    let nav_res = backend.navigate("https://example.com").await;
    assert!(nav_res.is_ok(), "Navigation failed: {:?}", nav_res.err());

    // Test Scanner Execution
    let scan_req = ScannerAction::Scan(oryn_engine::protocol::ScanRequest {
        max_elements: Some(10),
        include_hidden: false,
        monitor_changes: false,
        view_all: false,
        near: None,
        viewport_only: false,
    });

    let scan_res = backend.execute_scanner(scan_req).await;
    assert!(
        scan_res.is_ok(),
        "Scanner execution failed: {:?}",
        scan_res.err()
    );

    match scan_res.unwrap() {
        oryn_engine::protocol::ScannerProtocolResponse::Ok { data, .. } => {
            println!("Scanner response received: {:?}", data);
        }
        oryn_engine::protocol::ScannerProtocolResponse::Error { code, message, .. } => {
            panic!("Scanner returned error: {} - {}", code, message);
        }
    }

    // Test Screenshot (may fail in headless mode)
    let shot = backend.screenshot().await;
    match shot {
        Ok(bytes) => {
            assert!(!bytes.is_empty(), "Screenshot was empty");
            println!("Screenshot captured: {} bytes", bytes.len());
        }
        Err(e) => {
            // Screenshot not supported in headless COG - this is expected
            println!("Screenshot not available (expected in headless): {}", e);
        }
    }

    let _ = backend.close().await;
}

#[tokio::test]
#[serial]
#[ignore] // Requires weston/cog - run via ./scripts/run-tests.sh or in Docker
async fn test_embedded_navigation() {
    init_tracing();
    // Use port 8083 to avoid conflicts with parallel tests
    let mut backend = EmbeddedBackend::new_headless_on_port(8083);
    backend.launch().await.expect("Failed to launch backend");

    // Navigate to first page
    backend
        .navigate("https://example.com")
        .await
        .expect("Nav failed");

    // Test refresh
    let refresh_res = backend.refresh().await;
    assert!(
        refresh_res.is_ok(),
        "Refresh failed: {:?}",
        refresh_res.err()
    );
    println!("Refresh successful: {}", refresh_res.unwrap().url);

    // Test go_back (should work even with no history - just stays on same page)
    let back_res = backend.go_back().await;
    assert!(back_res.is_ok(), "Go back failed: {:?}", back_res.err());
    println!("Go back result: {}", back_res.unwrap().url);

    // Test go_forward
    let fwd_res = backend.go_forward().await;
    assert!(fwd_res.is_ok(), "Go forward failed: {:?}", fwd_res.err());
    println!("Go forward result: {}", fwd_res.unwrap().url);

    // Test cookies
    let cookies = backend.get_cookies().await.expect("Get cookies failed");
    println!("Cookies: {:?}", cookies);

    // Test tabs
    let tabs = backend.get_tabs().await.expect("Get tabs failed");
    assert!(!tabs.is_empty(), "Tabs should not be empty");
    println!("Tabs: {:?}", tabs);

    let _ = backend.close().await;
}
