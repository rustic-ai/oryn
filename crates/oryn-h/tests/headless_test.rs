use oryn_core::backend::Backend;
use oryn_core::protocol::{ScanRequest, ScannerRequest};
use oryn_h::backend::HeadlessBackend;
use serial_test::serial;
use std::path::Path;
use tokio::fs;

#[tokio::test]
#[serial]
async fn test_headless_lifecycle_and_scan() {
    tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::INFO)
        .try_init()
        .ok();

    // 1. Launch
    let mut backend = HeadlessBackend::new();
    // Use a custom routine if we want to enforce headless or special flags?
    // HeadlessBackend::launch defaults to no_sandbox which is good.

    // We wrap in a block to ensure cleanup if possible, though backend.close handles it.
    // However, if launch fails (no chrome), we might want to skip or fail gracefully?
    // For CI, we expect it to work or be skipped.
    match backend.launch().await {
        Ok(_) => {}
        Err(e) => {
            eprintln!("Failed to launch browser (is Chromium installed?): {}", e);
            // Optionally ignore test if environment lacks browser?
            // panic!("Browser launch failed");
            return;
        }
    }

    // 2. Navigate to data URL
    let html = "<html><head><title>Test Page</title></head><body><h1 id='h1'>Hello World</h1><button id='btn'>Click Me</button></body></html>";
    let url = format!("data:text/html,{}", html);

    let nav_res = backend.navigate(&url).await.expect("Navigation failed");
    assert_eq!(nav_res.title, "Test Page");

    // 3. Execute Scanner
    // This triggers injection
    let scan_req = ScannerRequest::Scan(ScanRequest {
        max_elements: None,
        monitor_changes: false,
        include_hidden: false,
        view_all: true,
    });

    let resp = backend
        .execute_scanner(scan_req)
        .await
        .expect("Scan failed");

    // Verify response
    if let oryn_core::protocol::ScannerProtocolResponse::Ok { data, .. } = resp {
        match *data {
            oryn_core::protocol::ScannerData::Scan(result)
            | oryn_core::protocol::ScannerData::ScanValidation(result) => {
                assert_eq!(result.page.title, "Test Page");
                // Find Button
                let btn = result
                    .elements
                    .iter()
                    .find(|e| e.element_type == "button" || e.text.as_deref() == Some("Click Me"));
                assert!(
                    btn.is_some(),
                    "Could not find Button element in scan results"
                );
            }
            _ => panic!("Expected Scan result, got: {:?}", data),
        }
    } else {
        panic!("Scanner returned error: {:?}", resp);
    }

    // 4. Test PDF Generation (using verify internal helper or expose it?)
    // HeadlessBackend exposes `get_client()`.
    if let Some(client) = backend.get_client() {
        let output_path = Path::new("test_output.pdf");
        let pdf_res = oryn_h::features::generate_pdf(&client.page, output_path).await;
        assert!(
            pdf_res.is_ok(),
            "PDF generation failed: {:?}",
            pdf_res.err()
        );

        assert!(output_path.exists());
        fs::remove_file(output_path).await.ok();
    } else {
        panic!("Client not available");
    }

    // 6. Test Backend Trait methods
    let cookies = backend.get_cookies().await.expect("Get cookies failed");
    // Since it's a data URL, there might be no cookies, but it shouldn't error
    println!("Cookies: {:?}", cookies);

    let tabs = backend.get_tabs().await.expect("Get tabs failed");
    assert!(!tabs.is_empty(), "Tabs should not be empty");
    assert!(tabs.iter().any(|t| t.url.contains("data:text/html")));

    let pdf_bytes = backend.pdf().await.expect("Trait PDF failed");
    assert!(!pdf_bytes.is_empty(), "PDF should not be empty");

    // 7. Close
    backend.close().await.expect("Close failed");
}
