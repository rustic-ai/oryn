use async_trait::async_trait;
use oryn_core::backend::{Backend, BackendError, NavigationResult};
use oryn_core::intent::definition::{
    ActionStep, ActionType, IntentDefinition, IntentTier, LoopDef, LoopStepWrapper, ParamType,
    ParameterDef, Step, TargetKind, TargetSpec, TryStepWrapper,
};
use oryn_core::intent::executor::IntentExecutor;
use oryn_core::intent::registry::IntentRegistry;
use oryn_core::intent::verifier::Verifier;
use oryn_core::protocol::{
    PageInfo, ScanResult, ScanStats, ScannerData, ScannerProtocolResponse, ScannerRequest,
    ScrollInfo, ViewportInfo,
};
use serde_json::json;
use std::collections::HashMap;

#[derive(Debug, Default)]
struct MockBackend {
    pub executed_requests: Vec<ScannerRequest>,
}

#[async_trait]
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
        Err(BackendError::NotSupported("navigate".into()))
    }

    async fn execute_scanner(
        &mut self,
        command: ScannerRequest,
    ) -> Result<ScannerProtocolResponse, BackendError> {
        self.executed_requests.push(command.clone());

        match &command {
            ScannerRequest::Scan(_) => {
                // Return empty scan result
                Ok(ScannerProtocolResponse::Ok {
                    data: Box::new(ScannerData::Scan(ScanResult {
                        page: PageInfo {
                            url: "test".into(),
                            title: "test".into(),
                            viewport: ViewportInfo::default(),
                            scroll: ScrollInfo::default(),
                        },
                        elements: vec![oryn_core::protocol::Element {
                            id: 1,
                            element_type: "input".into(),
                            role: None,
                            text: None,
                            label: None,
                            value: None,
                            placeholder: None,
                            selector: "#input".into(),
                            xpath: None,
                            rect: oryn_core::protocol::Rect {
                                x: 0.0,
                                y: 0.0,
                                width: 100.0,
                                height: 100.0,
                            },
                            attributes: HashMap::new(),
                            state: oryn_core::protocol::ElementState::default(),
                            children: vec![],
                        }],
                        stats: ScanStats {
                            total: 0,
                            scanned: 0,
                        },
                        patterns: None,
                        changes: None,
                    })),
                    warnings: vec![],
                })
            }
            ScannerRequest::Type(req) if req.text == "FAIL" => {
                // Simulate failure for "FAIL" text
                Err(BackendError::ScriptError("Simulated failure".into()))
            }
            _ => Ok(ScannerProtocolResponse::Ok {
                data: Box::new(ScannerData::Action(oryn_core::protocol::ActionResult {
                    success: true,
                    message: Some("Mock executed".into()),
                    navigation: None,
                })),
                warnings: vec![],
            }),
        }
    }

    async fn screenshot(&mut self) -> Result<Vec<u8>, BackendError> {
        Ok(vec![])
    }
}

#[tokio::test]
async fn test_executor_loop() {
    let mut backend = MockBackend::default();
    let registry = IntentRegistry::new();
    let verifier = Verifier;

    // Create intent with loop
    let intent = IntentDefinition {
        name: "test_loop".to_string(),
        version: "1.0".to_string(),
        tier: IntentTier::Loaded,
        triggers: Default::default(),
        parameters: vec![ParameterDef {
            name: "items".to_string(),
            param_type: ParamType::Array,
            required: true,
            default: None,
            description: "".into(),
        }],
        steps: vec![Step::Loop(LoopStepWrapper {
            loop_: LoopDef {
                over: "items".to_string(),
                as_var: "current_item".to_string(),
                max: 5,
                steps: vec![Step::Action(ActionStep {
                    action: ActionType::Type,
                    target: Some(TargetSpec {
                        kind: TargetKind::Id { id: 1 },
                        fallback: None,
                    }),
                    options: HashMap::from([("text".to_string(), json!("$current_item"))]),
                })],
            },
        })],
        success: None,
        failure: None,
        options: Default::default(),
    };

    let mut reg = registry;
    reg.register(intent);

    let mut executor = IntentExecutor::new(&mut backend, &reg, &verifier);

    let params = HashMap::from([("items".to_string(), json!(["A", "B", "C"]))]);

    let result = executor.execute("test_loop", params).await;
    assert!(result.is_ok(), "Executor failed: {:?}", result.err());

    let reqs = &backend.executed_requests;
    let type_reqs: Vec<_> = reqs
        .iter()
        .filter_map(|r| match r {
            ScannerRequest::Type(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect();

    assert_eq!(type_reqs, vec!["A", "B", "C"]);
}

#[tokio::test]
async fn test_executor_try() {
    let mut backend = MockBackend::default();
    let registry = IntentRegistry::new();
    let verifier = Verifier;

    // Create intent with try-catch
    let intent = IntentDefinition {
        name: "test_try".to_string(),
        version: "1.0".to_string(),
        tier: IntentTier::Loaded,
        triggers: Default::default(),
        parameters: vec![],
        steps: vec![Step::Try(TryStepWrapper {
            try_: oryn_core::intent::definition::TryDef {
                steps: vec![Step::Action(ActionStep {
                    action: ActionType::Type,
                    target: Some(TargetSpec {
                        kind: TargetKind::Id { id: 1 },
                        fallback: None,
                    }),
                    options: HashMap::from([
                        ("text".to_string(), json!("FAIL")), // Triggers backend error
                    ]),
                })],
                catch: vec![Step::Action(ActionStep {
                    action: ActionType::Type,
                    target: Some(TargetSpec {
                        kind: TargetKind::Id { id: 1 },
                        fallback: None,
                    }),
                    options: HashMap::from([("text".to_string(), json!("RECOVERED"))]),
                })],
            },
        })],
        success: None,
        failure: None,
        options: Default::default(),
    };

    let mut reg = registry;
    reg.register(intent);

    let mut executor = IntentExecutor::new(&mut backend, &reg, &verifier);

    let result = executor.execute("test_try", HashMap::new()).await;
    if let Err(e) = &result {
        panic!("Executor failed: {:?}", e);
    }
    assert!(result.is_ok()); // Intent should not fail because catch handled it

    let reqs = &backend.executed_requests;
    let type_reqs: Vec<_> = reqs
        .iter()
        .filter_map(|r| match r {
            ScannerRequest::Type(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect();

    // "FAIL" might be recorded BEFORE error is thrown depending on implementation?
    // Wait, backend receives "FAIL". It returns Err.
    // Executor gets Err. It stops Try block.
    // It executes Catch block. "RECOVERED".
    // "FAIL" request IS sent to backend. So it's in executed_requests.
    // So both "FAIL" and "RECOVERED" should be present.

    assert_eq!(type_reqs, vec!["FAIL", "RECOVERED"]);
}
