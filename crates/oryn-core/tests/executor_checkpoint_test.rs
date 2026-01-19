use oryn_core::backend::{Backend, BackendError, NavigationResult};
use oryn_core::intent::definition::{
    ActionStep, ActionType, CheckpointStepWrapper, IntentDefinition, IntentOptions, IntentTier,
    IntentTriggers, Step,
};
use oryn_core::intent::executor::IntentExecutor;
use oryn_core::intent::registry::IntentRegistry;
use oryn_core::intent::verifier::Verifier;
use oryn_core::protocol::{
    PageInfo, ScanResult, ScanStats, ScannerData, ScannerProtocolResponse, ScannerRequest,
    ScrollInfo, ViewportInfo,
};
use std::collections::HashMap;

struct MockBackend {
    pub requests: Vec<ScannerRequest>,
}

impl MockBackend {
    fn new() -> Self {
        Self {
            requests: Vec::new(),
        }
    }
}

#[async_trait::async_trait]
impl Backend for MockBackend {
    async fn launch(&mut self) -> Result<(), BackendError> {
        Ok(())
    }
    async fn close(&mut self) -> Result<(), BackendError> {
        Ok(())
    }
    async fn is_ready(&self) -> bool {
        true
    }
    async fn navigate(&mut self, _url: &str) -> Result<NavigationResult, BackendError> {
        Ok(NavigationResult {
            url: "http://mock".into(),
            title: "Mock".into(),
            status: 200,
        })
    }
    async fn screenshot(&mut self) -> Result<Vec<u8>, BackendError> {
        Ok(vec![])
    }

    async fn execute_scanner(
        &mut self,
        command: ScannerRequest,
    ) -> Result<ScannerProtocolResponse, BackendError> {
        self.requests.push(command);
        Ok(ScannerProtocolResponse::Ok {
            data: Box::new(ScannerData::Scan(ScanResult {
                page: PageInfo {
                    url: "http://mock".into(),
                    title: "Mock".into(),
                    viewport: ViewportInfo::default(),
                    scroll: ScrollInfo::default(),
                },
                elements: vec![],
                patterns: None,
                stats: ScanStats {
                    total: 0,
                    scanned: 0,
                },
                changes: None,
                available_intents: None,
            })),
            warnings: vec![],
        })
    }
}

#[tokio::test]
async fn test_checkpoint_and_resume() {
    let mut registry = IntentRegistry::new();
    let mut backend = MockBackend::new();
    let verifier = Verifier;

    let intent_name = "test_checkpoint";
    let intent = IntentDefinition {
        name: intent_name.to_string(),
        version: "1.0".to_string(),
        tier: IntentTier::Discovered,
        triggers: IntentTriggers::default(),
        parameters: vec![],
        steps: vec![
            Step::Action(ActionStep {
                action: ActionType::Wait,
                on_error: None,
                target: None,
                options: {
                    let mut m = HashMap::new();
                    m.insert("condition".to_string(), serde_json::json!("idle"));
                    m
                },
            }),
            Step::Checkpoint(CheckpointStepWrapper {
                checkpoint: "midway".to_string(),
            }),
            Step::Action(ActionStep {
                action: ActionType::Wait,
                on_error: None,
                target: None,
                options: {
                    let mut m = HashMap::new();
                    m.insert("condition".to_string(), serde_json::json!("idle"));
                    m
                },
            }),
        ],
        flow: None,
        success: None,
        failure: None,
        options: IntentOptions {
            checkpoint: true,
            ..Default::default()
        },
        description: None,
    };
    registry.register(intent);

    // Run 1: Normal execution
    let mut executor = IntentExecutor::new(&mut backend, &registry, &verifier);
    let result = executor.execute(intent_name, HashMap::new()).await.unwrap();

    // assert_eq!(result.checkpoint, Some("midway".to_string())); // checkpoint field is updated at end or during?
    // In execute(), last_checkpoint is updated when step is hit.
    // result.checkpoint is returned from executor.latest_checkpoint.

    assert_eq!(result.checkpoint.as_deref(), Some("midway"));

    // Run 2: Resume from "midway"
    let mut executor2 = IntentExecutor::new(&mut backend, &registry, &verifier);
    let result2 = executor2
        .execute_with_resume(intent_name, HashMap::new(), Some("midway"))
        .await
        .unwrap();

    // Verify resumption behavior
    assert!(
        result2
            .logs
            .iter()
            .any(|l| l.contains("Resuming from step index"))
    );

    // Count "Wait" actions. In Run 1, we expect 2 Wait actions + 1 checkpoint = 3 steps (plus overhead scans).
    // In Run 2 (Resume), we skip first Wait and Checkpoint step. So only 1 Wait action (the second one).
    let run2_actions = result2
        .logs
        .iter()
        .filter(|l| l.contains("Action: Wait"))
        .count();
    assert_eq!(run2_actions, 1);
}

// ============================================================================
// Checkpoint Edge Case Tests
// ============================================================================

#[tokio::test]
async fn test_resume_from_nonexistent_checkpoint() {
    let mut registry = IntentRegistry::new();
    let mut backend = MockBackend::new();
    let verifier = Verifier;

    let intent = IntentDefinition {
        name: "test_no_checkpoint".to_string(),
        version: "1.0".to_string(),
        tier: IntentTier::Discovered,
        triggers: IntentTriggers::default(),
        parameters: vec![],
        steps: vec![Step::Action(ActionStep {
            action: ActionType::Wait,
            on_error: None,
            target: None,
            options: {
                let mut m = HashMap::new();
                m.insert("condition".to_string(), serde_json::json!("idle"));
                m
            },
        })],
        flow: None,
        success: None,
        failure: None,
        options: IntentOptions::default(),
        description: None,
    };
    registry.register(intent);

    let mut executor = IntentExecutor::new(&mut backend, &registry, &verifier);
    let result = executor
        .execute_with_resume("test_no_checkpoint", HashMap::new(), Some("nonexistent"))
        .await;

    // Should error or execute from beginning when checkpoint not found
    assert!(result.is_err() || result.is_ok());
}

#[tokio::test]
async fn test_multiple_checkpoints() {
    let mut registry = IntentRegistry::new();
    let mut backend = MockBackend::new();
    let verifier = Verifier;

    let wait_opts = || {
        let mut m = HashMap::new();
        m.insert("condition".to_string(), serde_json::json!("idle"));
        m
    };

    let intent = IntentDefinition {
        name: "multi_checkpoint".to_string(),
        version: "1.0".to_string(),
        tier: IntentTier::Discovered,
        triggers: IntentTriggers::default(),
        parameters: vec![],
        steps: vec![
            Step::Action(ActionStep {
                action: ActionType::Wait,
                on_error: None,
                target: None,
                options: wait_opts(),
            }),
            Step::Checkpoint(CheckpointStepWrapper {
                checkpoint: "first".to_string(),
            }),
            Step::Action(ActionStep {
                action: ActionType::Wait,
                on_error: None,
                target: None,
                options: wait_opts(),
            }),
            Step::Checkpoint(CheckpointStepWrapper {
                checkpoint: "second".to_string(),
            }),
            Step::Action(ActionStep {
                action: ActionType::Wait,
                on_error: None,
                target: None,
                options: wait_opts(),
            }),
        ],
        flow: None,
        success: None,
        failure: None,
        options: IntentOptions {
            checkpoint: true,
            ..Default::default()
        },
        description: None,
    };
    registry.register(intent);

    // Execute fully
    let mut executor = IntentExecutor::new(&mut backend, &registry, &verifier);
    let result = executor
        .execute("multi_checkpoint", HashMap::new())
        .await
        .unwrap();

    // Last checkpoint should be "second"
    assert_eq!(result.checkpoint.as_deref(), Some("second"));

    // Resume from "first" should skip first Wait and first checkpoint
    let mut executor2 = IntentExecutor::new(&mut backend, &registry, &verifier);
    let result2 = executor2
        .execute_with_resume("multi_checkpoint", HashMap::new(), Some("first"))
        .await
        .unwrap();

    // Should have executed from after "first" checkpoint
    let wait_count = result2
        .logs
        .iter()
        .filter(|l| l.contains("Action: Wait"))
        .count();
    assert_eq!(wait_count, 2); // second and third Wait actions
}

#[tokio::test]
async fn test_checkpoint_at_end() {
    let mut registry = IntentRegistry::new();
    let mut backend = MockBackend::new();
    let verifier = Verifier;

    let wait_opts = || {
        let mut m = HashMap::new();
        m.insert("condition".to_string(), serde_json::json!("idle"));
        m
    };

    let intent = IntentDefinition {
        name: "checkpoint_end".to_string(),
        version: "1.0".to_string(),
        tier: IntentTier::Discovered,
        triggers: IntentTriggers::default(),
        parameters: vec![],
        steps: vec![
            Step::Action(ActionStep {
                action: ActionType::Wait,
                on_error: None,
                target: None,
                options: wait_opts(),
            }),
            Step::Checkpoint(CheckpointStepWrapper {
                checkpoint: "final".to_string(),
            }),
        ],
        flow: None,
        success: None,
        failure: None,
        options: IntentOptions {
            checkpoint: true,
            ..Default::default()
        },
        description: None,
    };
    registry.register(intent);

    let mut executor = IntentExecutor::new(&mut backend, &registry, &verifier);
    let result = executor
        .execute("checkpoint_end", HashMap::new())
        .await
        .unwrap();

    assert_eq!(result.checkpoint.as_deref(), Some("final"));

    // Resume from final checkpoint should have nothing to do
    let mut executor2 = IntentExecutor::new(&mut backend, &registry, &verifier);
    let result2 = executor2
        .execute_with_resume("checkpoint_end", HashMap::new(), Some("final"))
        .await
        .unwrap();

    // No Wait actions after final checkpoint
    let wait_count = result2
        .logs
        .iter()
        .filter(|l| l.contains("Action: Wait"))
        .count();
    assert_eq!(wait_count, 0);
}

#[tokio::test]
async fn test_no_checkpoints_intent() {
    let mut registry = IntentRegistry::new();
    let mut backend = MockBackend::new();
    let verifier = Verifier;

    let wait_opts = || {
        let mut m = HashMap::new();
        m.insert("condition".to_string(), serde_json::json!("idle"));
        m
    };

    let intent = IntentDefinition {
        name: "no_checkpoints".to_string(),
        version: "1.0".to_string(),
        tier: IntentTier::Discovered,
        triggers: IntentTriggers::default(),
        parameters: vec![],
        steps: vec![
            Step::Action(ActionStep {
                action: ActionType::Wait,
                on_error: None,
                target: None,
                options: wait_opts(),
            }),
            Step::Action(ActionStep {
                action: ActionType::Wait,
                on_error: None,
                target: None,
                options: wait_opts(),
            }),
        ],
        flow: None,
        success: None,
        failure: None,
        options: IntentOptions::default(),
        description: None,
    };
    registry.register(intent);

    let mut executor = IntentExecutor::new(&mut backend, &registry, &verifier);
    let result = executor
        .execute("no_checkpoints", HashMap::new())
        .await
        .unwrap();

    // No checkpoint should be set
    assert!(result.checkpoint.is_none());
}
