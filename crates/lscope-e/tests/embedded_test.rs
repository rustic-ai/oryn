use lscope_core::backend::Backend;
use lscope_e::backend::EmbeddedBackend;
use lscope_e::cog;

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
            // 3. Navigate
            let res = backend.navigate("https://example.com").await;
            assert!(res.is_ok());

            // 4. Close
            let close_res = backend.close().await;
            assert!(close_res.is_ok());
        }
        Err(e) => {
            eprintln!("Skipping test: Could not connect to WebDriver: {}", e);
        }
    }
}
