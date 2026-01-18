use async_trait::async_trait;
use oryn_core::backend::{Backend, BackendError, NavigationResult};
use oryn_core::formatter;
use oryn_core::intent::definition::{
    ActionStep, ActionType, IntentDefinition, IntentTier, IntentTriggers, Step,
};
use oryn_core::intent::executor::{IntentExecutor, IntentStatus};
use oryn_core::intent::registry::IntentRegistry;
use oryn_core::intent::verifier::Verifier;
use oryn_core::protocol::{
    AvailabilityStatus, DetectedPatterns, IntentAvailability, PageInfo, ScanResult, ScanStats,
    ScannerData, ScannerProtocolResponse, ScrollInfo, ViewportInfo,
};
use std::collections::HashMap;

// Mock Backend
struct MockBackend {
    scan_result: ScanResult,
}

#[async_trait]
impl Backend for MockBackend {
    async fn navigate(&mut self, _url: &str) -> Result<NavigationResult, BackendError> {
        Ok(NavigationResult {
            url: _url.to_string(),
            title: "Mock".to_string(),
            status: 200,
        })
    }
    async fn execute_scanner(
        &mut self,
        _req: oryn_core::protocol::ScannerRequest,
    ) -> Result<ScannerProtocolResponse, BackendError> {
        Ok(ScannerProtocolResponse::Ok {
            data: Box::new(ScannerData::Scan(self.scan_result.clone())),
            warnings: vec![],
        })
    }

    async fn launch(&mut self) -> Result<(), BackendError> {
        Ok(())
    }
    async fn close(&mut self) -> Result<(), BackendError> {
        Ok(())
    }
    async fn is_ready(&self) -> bool {
        true
    }
    async fn screenshot(&mut self) -> Result<Vec<u8>, BackendError> {
        Ok(vec![])
    }
}

fn mock_scan_result(url: &str) -> ScanResult {
    ScanResult {
        page: PageInfo {
            url: url.to_string(),
            title: "Test Page".to_string(),
            viewport: ViewportInfo::default(),
            scroll: ScrollInfo::default(),
        },
        elements: vec![],
        stats: ScanStats {
            total: 0,
            scanned: 0,
        },
        patterns: Some(DetectedPatterns::default()),
        changes: None,
        available_intents: None,
    }
}

#[test]
fn test_formatter_available_intents() {
    let mut scan = mock_scan_result("https://example.com");
    // Simulate populated available intents
    scan.available_intents = Some(vec![
        IntentAvailability {
            name: "login".to_string(),
            status: AvailabilityStatus::Ready,
            parameters: vec!["username".to_string()],
            trigger_reason: None,
        },
        IntentAvailability {
            name: "checkout".to_string(),
            status: AvailabilityStatus::MissingPattern,
            parameters: vec![],
            trigger_reason: Some("Missing pattern: cart".to_string()),
        },
    ]);

    let data = ScannerData::Scan(scan);
    let resp = ScannerProtocolResponse::Ok {
        data: Box::new(data),
        warnings: vec![],
    };

    let output = formatter::format_response(&resp);
    assert!(output.contains("Available Intents:"));
    assert!(output.contains("🟢 login (username)"));
    assert!(output.contains("🔴 checkout [Missing pattern: cart]"));
}

#[tokio::test]
async fn test_executor_partial_success() {
    // 1. Setup Mock Backend
    let mut backend = MockBackend {
        scan_result: mock_scan_result("https://example.com"),
    };

    // 2. Setup Registry with a multi-step intent
    let mut registry = IntentRegistry::new();
    let intent = IntentDefinition {
        name: "test_partial".to_string(),
        version: "1.0".to_string(),
        description: None,
        tier: IntentTier::BuiltIn,
        triggers: IntentTriggers::default(),
        parameters: vec![],
        steps: vec![
            Step::Action(ActionStep {
                action: ActionType::Wait, // Should succeed (dummy backend scans ok)
                target: None,
                options: {
                    let mut m = HashMap::new();
                    m.insert("condition".to_string(), serde_json::json!("load"));
                    m
                },
            }),
            Step::Action(ActionStep {
                action: ActionType::Wait,
                target: None,
                options: {
                    let mut m = HashMap::new();
                    m.insert(
                        "condition".to_string(),
                        serde_json::json!("uknown_condition_to_fail"),
                    );
                    m
                },
            }),
        ],
        success: None,
        failure: None,
        options: Default::default(),
    };
    registry.register(intent);

    // 3. Setup Executor
    let verifier = Verifier::new();
    let mut executor = IntentExecutor::new(&mut backend, &registry, &verifier);

    // 4. Execute
    let result = executor.execute("test_partial", HashMap::new()).await;

    // 5. Verify Result
    match result {
        Ok(res) => {
            if let IntentStatus::PartialSuccess { completed, total } = res.status {
                assert_eq!(completed, 1);
                assert_eq!(total, 2);
            } else {
                panic!("Expected PartialSuccess, got {:?}", res.status);
            }
        }
        Err(e) => panic!(
            "Executor failed hard instead of returning partial success: {}",
            e
        ),
    }
}
