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
                target: None,
                options: {
                    let mut m = HashMap::new();
                    m.insert("condition".to_string(), serde_json::json!("idle"));
                    m
                },
            }),
        ],
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
