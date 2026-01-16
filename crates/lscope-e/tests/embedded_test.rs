//! Embedded backend integration tests
//!
//! These tests launch real COG browser instances via WPEWebDriver.
//! Run with: `cargo test -p lscope-e -- --test-threads=1`
//!
//! Note: Tests must run sequentially (--test-threads=1) to avoid
//! resource contention with browser processes and network requests.

use lscope_core::backend::Backend;
use lscope_core::protocol::ScannerRequest;
use lscope_e::backend::EmbeddedBackend;

#[tokio::test]
async fn test_embedded_lifecycle() {
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
async fn test_embedded_features() {
    // Use port 8082 to avoid conflicts with parallel tests
    let mut backend = EmbeddedBackend::new_headless_on_port(8082);

    backend.launch().await.expect("Failed to launch backend");

    // Navigate to a real page first
    let nav_res = backend.navigate("https://example.com").await;
    assert!(nav_res.is_ok(), "Navigation failed: {:?}", nav_res.err());

    // Test Scanner Execution
    let scan_req = ScannerRequest::Scan(lscope_core::protocol::ScanRequest {
        max_elements: Some(10),
        include_hidden: false,
        monitor_changes: false,
        view_all: false,
    });

    let scan_res = backend.execute_scanner(scan_req).await;
    assert!(
        scan_res.is_ok(),
        "Scanner execution failed: {:?}",
        scan_res.err()
    );

    match scan_res.unwrap() {
        lscope_core::protocol::ScannerProtocolResponse::Ok { data, .. } => {
            println!("Scanner response received: {:?}", data);
        }
        lscope_core::protocol::ScannerProtocolResponse::Error { code, message, .. } => {
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
async fn test_embedded_navigation() {
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

    let _ = backend.close().await;
}
