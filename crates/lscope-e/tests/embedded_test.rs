use lscope_core::backend::Backend;
use lscope_core::protocol::ScannerRequest;
use lscope_e::backend::EmbeddedBackend;
use lscope_e::cog;
use tokio;

#[tokio::test]
#[ignore = "Requires running WebDriver instance"]
async fn test_embedded_lifecycle() {
    // 1. Setup
    // Assumes default COG port 8080
    let url = cog::default_cog_url();
    let mut backend = EmbeddedBackend::new(url);

    // 2. Launch
    // This will fail if no WebDriver is running.
    match backend.launch().await {
        Ok(_) => {
            println!("Connected to WebDriver.");

            // 3. Navigate
            let res = backend.navigate("https://example.com").await;
            assert!(res.is_ok(), "Navigation failed: {:?}", res.err());
            let nav_res = res.unwrap();
            println!("Navigated to: {}", nav_res.url);

            // 4. Close
            let close_res = backend.close().await;
            assert!(close_res.is_ok(), "Close failed: {:?}", close_res.err());
        }
        Err(e) => {
            eprintln!("Skipping test: Could not connect to WebDriver: {}", e);
        }
    }
}

#[tokio::test]
#[ignore = "Requires running WebDriver instance"]
async fn test_embedded_features() {
    let url = cog::default_cog_url();
    let mut backend = EmbeddedBackend::new(url);

    if let Err(e) = backend.launch().await {
        eprintln!("Skipping test: Could not connect to WebDriver: {}", e);
        return;
    }

    // 1. Test Navigation Error handling
    // Try navigating to a non-existent local port to force an error
    let bad_res = backend.navigate("http://localhost:59999").await;
    // Note: Behavior depends on browser, but usually it returns Ok with error page or Err
    // We just check it doesn't panic
    println!("Navigation to invalid URL result: {:?}", bad_res);

    // 2. Test Scanner Execution
    // Navigate to a real page first
    let _ = backend.navigate("https://example.com").await;

    // Simple scan
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

    // 3. Test Screenshot
    let shot = backend.screenshot().await;
    assert!(shot.is_ok(), "Screenshot failed: {:?}", shot.err());
    let bytes = shot.unwrap();
    assert!(!bytes.is_empty(), "Screenshot was empty");
    println!("Screenshot captured: {} bytes", bytes.len());

    let _ = backend.close().await;
}
