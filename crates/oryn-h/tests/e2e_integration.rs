use oryn_engine::backend::Backend;
use oryn_engine::parser::Parser;
use oryn_engine::translator::translate;
use oryn_h::backend::HeadlessBackend;
use serial_test::serial;

#[tokio::test]
#[serial]
async fn test_full_flow_search() {
    // 1. Setup Backend (Headless)
    let _chrome_path =
        std::env::var("CHROME_BIN").unwrap_or_else(|_| "/usr/bin/google-chrome".to_string());
    // Fallback for CI or local dev if standard path fails
    // Note: requires chromiumoxide compatible browser

    // We actually use the HeadlessBackend logic which auto-detects if path is None
    // But for test stability we might want to check if browser exists

    let mut backend = HeadlessBackend::new();

    match backend.launch().await {
        Ok(_) => {
            // 2. Parse Intent Command
            let input = "goto \"https://example.com\"";
            let mut parser = Parser::new(input);
            let commands = parser.parse().expect("Failed to parse goto");
            assert_eq!(commands.len(), 1);

            // 3. Execute Navigation (Special case in translator usually, but Backend handles it)
            // The translator returns ScannerRequest, but Navigation is distinct in Backend trait
            // Let's manually handle navigation for this test since we know it's a GoTo
            if let oryn_engine::command::Command::GoTo(url) = &commands[0] {
                let res = backend.navigate(url).await.expect("Navigation failed");
                assert!(res.url.contains("example.com"));
            }

            // 4. Interact: Scan first to find IDs
            let scan_cmd = oryn_engine::command::Command::Observe(std::collections::HashMap::new());
            let scan_req = translate(&scan_cmd).expect("Failed to translate scan");

            let scan_res = backend
                .execute_scanner(scan_req)
                .await
                .expect("Scan failed");

            // 5. Find an element to click (e.g., h1)
            let mut target_id = None;
            if let oryn_engine::protocol::ScannerProtocolResponse::Ok { data, .. } = scan_res
                && let oryn_engine::protocol::ScannerData::Scan(scan_data) = data.as_ref()
            {
                // Just pick the first element or strictly find h1
                if let Some(el) = scan_data.elements.first() {
                    println!("Found element to click: {:?}", el);
                    target_id = Some(el.id);
                }
            }

            if let Some(id) = target_id {
                let click_cmd = oryn_engine::command::Command::Click(
                    oryn_engine::command::Target::Id(id as usize),
                    std::collections::HashMap::new(),
                );
                let click_req = translate(&click_cmd).expect("Failed to translate click");
                let click_res = backend
                    .execute_scanner(click_req)
                    .await
                    .expect("Click failed");
                println!("Click result: {:?}", click_res);
            } else {
                println!("No elements found to click, skipping click test");
            }

            backend.close().await.expect("Failed to close");
        }
        Err(e) => {
            eprintln!("Skipping test: Headless browser not available: {}", e);
        }
    }
}

#[tokio::test]
#[serial]
async fn test_error_handling() {
    let mut backend = HeadlessBackend::new();
    if backend.launch().await.is_err() {
        return; // Skip if no browser
    }

    // Test 1: Click non-existent ID
    // ID 999999 likely doesn't exist
    let click_cmd = oryn_engine::command::Command::Click(
        oryn_engine::command::Target::Id(999999),
        std::collections::HashMap::new(),
    );
    let click_req = translate(&click_cmd).expect("Translation failed");

    // navigate first to have a context
    backend
        .navigate("data:text/html,<html><body></body></html>")
        .await
        .ok();

    let res = backend.execute_scanner(click_req).await;
    match res {
        Ok(oryn_engine::protocol::ScannerProtocolResponse::Error { code, .. }) => {
            println!("Got expected error: {}", code);
            assert!(code == "ELEMENT_NOT_FOUND" || code == "EXECUTION_ERROR");
        }
        Ok(r) => panic!("Expected error, got: {:?}", r),
        Err(e) => panic!("Backend error: {}", e), // Should return protocol error, not backend error ideally
    }

    backend.close().await.ok();
}

#[tokio::test]
#[serial]
async fn test_interaction_type() {
    let mut backend = HeadlessBackend::new();
    if backend.launch().await.is_err() {
        return;
    }

    backend
        .navigate("data:text/html,<html><body><input id='inp' type='text'></body></html>")
        .await
        .expect("Nav failed");

    // Scan to find ID
    let scan_req = translate(&oryn_engine::command::Command::Observe(
        std::collections::HashMap::new(),
    ))
    .unwrap();
    let scan_res = backend.execute_scanner(scan_req).await.unwrap();

    let mut input_id = None;
    if let oryn_engine::protocol::ScannerProtocolResponse::Ok { data, .. } = scan_res
        && let oryn_engine::protocol::ScannerData::Scan(scan_data) = data.as_ref()
    {
        for el in &scan_data.elements {
            if el.attributes.get("id").map(|s| s.as_str()) == Some("inp") {
                input_id = Some(el.id);
                break;
            }
        }
    }

    if let Some(id) = input_id {
        let type_cmd = oryn_engine::command::Command::Type(
            oryn_engine::command::Target::Id(id as usize),
            "Execute Order 66".to_string(),
            std::collections::HashMap::new(),
        );
        let type_req = translate(&type_cmd).unwrap();
        let type_res = backend.execute_scanner(type_req).await.unwrap();
        match type_res {
            oryn_engine::protocol::ScannerProtocolResponse::Ok { .. } => println!("Type success"),
            _ => panic!("Type failed: {:?}", type_res),
        }
    } else {
        panic!("Input element not found");
    }

    backend.close().await.ok();
}
