use async_trait::async_trait;
use oryn_engine::backend::{Backend, BackendError, NavigationResult};
use oryn_engine::intent::definition::{
    ActionStep, ActionType, BranchDef, BranchStepWrapper, Condition, IntentDefinition, IntentTier,
    IntentTriggers, LoopDef, LoopStepWrapper, Step,
};
use oryn_engine::intent::executor::{IntentExecutor, IntentStatus};
use oryn_engine::intent::registry::IntentRegistry;
use oryn_engine::intent::verifier::Verifier;
use oryn_engine::protocol::{
    PageInfo, ScanResult, ScanStats, ScannerData, ScannerProtocolResponse, ScrollInfo, ViewportInfo,
};
use serde_json::{Value, json};
use std::collections::HashMap;

// Mock Backend
struct MockBackend {
    scan_result: ScanResult,
    calls: Vec<String>,
}

#[async_trait]
impl Backend for MockBackend {
    async fn navigate(&mut self, _url: &str) -> Result<NavigationResult, BackendError> {
        self.calls.push(format!("navigate: {}", _url));
        Ok(NavigationResult {
            url: _url.to_string(),
            title: "Mock".to_string(),
            status: 200,
        })
    }
    async fn execute_scanner(
        &mut self,
        _req: oryn_engine::protocol::ScannerRequest,
    ) -> Result<ScannerProtocolResponse, BackendError> {
        // self.calls.push(format!("scanner: {:?}", req));
        // Logging entire req might be verbose/hard to match.
        // Let's just return success.
        Ok(ScannerProtocolResponse::Ok {
            data: Box::new(ScannerData::Scan(Box::new(self.scan_result.clone()))),
            warnings: vec![],
        })
    }

    async fn execute_script(&mut self, script: &str) -> Result<Value, BackendError> {
        self.calls.push(format!("script: {}", script));
        Ok(Value::Null)
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

fn mock_scan_result() -> ScanResult {
    ScanResult {
        page: PageInfo {
            url: "https://example.com".to_string(),
            title: "Test Page".to_string(),
            viewport: ViewportInfo::default(),
            scroll: ScrollInfo::default(),
        },
        elements: vec![],
        stats: ScanStats {
            total: 0,
            scanned: 0,
        },
        patterns: None,
        changes: None,
        available_intents: None,
    }
}

// Helper to create simple intent
fn create_intent(name: &str, steps: Vec<Step>) -> IntentDefinition {
    IntentDefinition {
        name: name.to_string(),
        version: "1.0".to_string(),
        description: None,
        tier: IntentTier::BuiltIn,
        triggers: IntentTriggers::default(),
        parameters: vec![],
        steps,
        flow: None,
        success: None,
        failure: None,
        options: Default::default(),
    }
}

#[tokio::test]
async fn test_branch_logic() {
    let mut backend = MockBackend {
        scan_result: mock_scan_result(),
        calls: vec![],
    };
    let mut registry = IntentRegistry::new();

    // Condition that is ALWAYS TRUE for this mock: UrlMatches(".*")
    let true_cond = Condition::UrlMatches(".*".to_string());

    let steps = vec![Step::Branch(BranchStepWrapper {
        branch: BranchDef {
            condition: true_cond,
            then_steps: vec![Step::Action(ActionStep {
                action: ActionType::Execute,
                on_error: None,
                target: None,
                options: {
                    let mut m = HashMap::new();
                    m.insert("script".to_string(), json!("console.log('then')"));
                    m
                },
            })],
            else_steps: vec![Step::Action(ActionStep {
                action: ActionType::Execute,
                on_error: None,
                target: None,
                options: {
                    let mut m = HashMap::new();
                    m.insert("script".to_string(), json!("console.log('else')"));
                    m
                },
            })],
        },
    })];

    registry.register(create_intent("test_branch", steps));
    let verifier = Verifier::new();
    let mut executor = IntentExecutor::new(&mut backend, &registry, &verifier);

    let res = executor
        .execute("test_branch", HashMap::new())
        .await
        .unwrap();
    assert_eq!(res.status, IntentStatus::Success);

    // Check backend calls
    assert!(backend.calls.iter().any(|c| c.contains("then")));
    assert!(!backend.calls.iter().any(|c| c.contains("else")));
}

#[tokio::test]
async fn test_loop_logic() {
    let mut backend = MockBackend {
        scan_result: mock_scan_result(),
        calls: vec![],
    };
    let mut registry = IntentRegistry::new();

    let steps = vec![Step::Loop(LoopStepWrapper {
        loop_: LoopDef {
            over: "3".to_string(), // treated as range 0..3
            as_var: "i".to_string(),
            max: 10,
            steps: vec![Step::Action(ActionStep {
                action: ActionType::Execute,
                on_error: None,
                target: None,
                options: {
                    let mut m = HashMap::new();
                    m.insert("script".to_string(), json!("iter"));
                    m
                },
            })],
        },
    })];

    registry.register(create_intent("test_loop", steps));
    let verifier = Verifier::new();
    let mut executor = IntentExecutor::new(&mut backend, &registry, &verifier);

    executor.execute("test_loop", HashMap::new()).await.unwrap();

    // Check backend calls count
    let count = backend.calls.iter().filter(|c| c.contains("iter")).count();
    assert_eq!(count, 3);
}

#[tokio::test]
async fn test_intent_composition() {
    let mut backend = MockBackend {
        scan_result: mock_scan_result(),
        calls: vec![],
    };
    let mut registry = IntentRegistry::new();

    // Inner Intent
    let inner_steps = vec![Step::Action(ActionStep {
        action: ActionType::Execute,
        on_error: None,
        target: None,
        options: {
            let mut m = HashMap::new();
            m.insert("script".to_string(), json!("inner"));
            m
        },
    })];
    registry.register(create_intent("inner", inner_steps));

    // Outer Intent calls Inner
    let outer_steps = vec![Step::Action(ActionStep {
        action: ActionType::Intent,
        on_error: None,
        target: None,
        options: {
            let mut m = HashMap::new();
            m.insert("name".to_string(), json!("inner"));
            m
        },
    })];
    registry.register(create_intent("outer", outer_steps));

    let verifier = Verifier::new();
    let mut executor = IntentExecutor::new(&mut backend, &registry, &verifier);

    executor.execute("outer", HashMap::new()).await.unwrap();

    assert!(backend.calls.iter().any(|c| c.contains("inner")));
}
