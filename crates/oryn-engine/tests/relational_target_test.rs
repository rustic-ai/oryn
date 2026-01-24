use async_trait::async_trait;
use oryn_engine::backend::{Backend, BackendError, NavigationResult};
use oryn_engine::intent::definition::{
    ActionStep, ActionType, IntentDefinition, IntentTier, Step, TargetKind, TargetSpec,
};
use oryn_engine::intent::executor::IntentExecutor;
use oryn_engine::intent::registry::IntentRegistry;
use oryn_engine::intent::verifier::Verifier;
use oryn_engine::protocol::{
    Element, ElementState, PageInfo, Rect, ScanResult, ScanStats, ScannerData,
    ScannerProtocolResponse, ScannerRequest, ScrollInfo, ViewportInfo,
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
                Ok(ScannerProtocolResponse::Ok {
                    data: Box::new(ScannerData::Scan(Box::new(ScanResult {
                        page: PageInfo {
                            url: "test".into(),
                            title: "Relational Test".into(),
                            viewport: ViewportInfo::default(),
                            scroll: ScrollInfo::default(),
                        },
                        elements: vec![
                            // 1. Label "Username"
                            Element {
                                id: 1,
                                element_type: "label".into(),
                                role: None,
                                text: Some("Username".into()),
                                label: None,
                                value: None,
                                placeholder: None,
                                selector: "#label-user".into(),
                                xpath: None,
                                rect: Rect {
                                    x: 10.0,
                                    y: 10.0,
                                    width: 50.0,
                                    height: 20.0,
                                }, // (10,10) -> (60,30)
                                attributes: HashMap::new(),
                                state: ElementState::default(),
                                children: vec![],
                            },
                            // 2. Input "username_input" (to the right of label)
                            Element {
                                id: 2,
                                element_type: "input".into(),
                                role: None,
                                text: None,
                                label: None,
                                value: None,
                                placeholder: None,
                                selector: "#input-user".into(),
                                xpath: None,
                                // Right of label (x: 60+)
                                rect: Rect {
                                    x: 70.0,
                                    y: 10.0,
                                    width: 100.0,
                                    height: 20.0,
                                },
                                attributes: HashMap::new(),
                                state: ElementState::default(),
                                children: vec![],
                            },
                            // 3. Container "Form"
                            Element {
                                id: 3,
                                element_type: "form".into(),
                                role: None,
                                text: None,
                                label: None,
                                value: None,
                                placeholder: None,
                                selector: "#form".into(),
                                xpath: None,
                                rect: Rect {
                                    x: 0.0,
                                    y: 0.0,
                                    width: 200.0,
                                    height: 200.0,
                                },
                                attributes: HashMap::new(),
                                state: ElementState::default(),
                                children: vec![],
                            },
                        ],
                        stats: ScanStats {
                            total: 0,
                            scanned: 0,
                        },
                        patterns: None,
                        changes: None,
                        available_intents: None,
                    }))),
                    warnings: vec![],
                })
            }
            _ => Ok(ScannerProtocolResponse::Ok {
                data: Box::new(ScannerData::Action(oryn_engine::protocol::ActionResult {
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
async fn test_relational_after() {
    let mut backend = MockBackend::default();
    let registry = IntentRegistry::new();
    let verifier = Verifier;

    // Intent: Type into input AFTER "Username"
    let intent = IntentDefinition {
        name: "test_after".to_string(),
        version: "1.0".to_string(),
        tier: IntentTier::Loaded,
        triggers: Default::default(),
        parameters: vec![],
        steps: vec![Step::Action(ActionStep {
            action: ActionType::Type,
            on_error: None,
            target: Some(TargetSpec {
                kind: TargetKind::After {
                    after: Box::new(TargetSpec {
                        kind: TargetKind::Selector {
                            selector: "input".into(),
                        },
                        fallback: None,
                    }),
                    anchor: Box::new(TargetSpec {
                        kind: TargetKind::Text {
                            text: "Username".into(),
                            match_type: Default::default(),
                        },
                        fallback: None,
                    }),
                },
                fallback: None,
            }),

            options: HashMap::from([("text".to_string(), json!("myuser"))]),
        })],
        flow: None,
        success: None,
        failure: None,
        options: Default::default(),
        description: None,
    };

    let mut reg = registry;
    reg.register(intent);

    let mut executor = IntentExecutor::new(&mut backend, &reg, &verifier);
    let result = executor.execute("test_after", HashMap::new()).await;
    assert!(result.is_ok(), "Executor failed: {:?}", result.err());

    // Check request: Should identify ID 2 (input)
    let reqs = &backend.executed_requests;
    let click_req = reqs
        .iter()
        .find(|r| matches!(r, ScannerRequest::Type(_)))
        .expect("No Type request");

    if let ScannerRequest::Type(cmd) = click_req {
        // ID 2 is the input
        assert_eq!(cmd.id, 2);
    }
}

#[tokio::test]
async fn test_relational_inside() {
    let mut backend = MockBackend::default();
    let registry = IntentRegistry::new();
    let verifier = Verifier;

    // Intent: Type into input INSIDE form
    let intent = IntentDefinition {
        name: "test_inside".to_string(),
        version: "1.0".to_string(),
        tier: IntentTier::Loaded,
        triggers: Default::default(),
        parameters: vec![],
        steps: vec![Step::Action(ActionStep {
            action: ActionType::Type,
            on_error: None,
            target: Some(TargetSpec {
                kind: TargetKind::Inside {
                    inside: Box::new(TargetSpec {
                        kind: TargetKind::Role {
                            role: "input".into(),
                        },
                        fallback: None,
                    }),
                    container: Box::new(TargetSpec {
                        kind: TargetKind::Id { id: 3 },
                        fallback: None,
                    }),
                },
                fallback: None,
            }),

            options: HashMap::from([("text".to_string(), json!("inside"))]),
        })],
        flow: None,
        success: None,
        failure: None,
        options: Default::default(),
        description: None,
    };

    let mut reg = registry;
    reg.register(intent);

    let mut executor = IntentExecutor::new(&mut backend, &reg, &verifier);
    let result = executor.execute("test_inside", HashMap::new()).await;
    assert!(result.is_ok());

    let reqs = &backend.executed_requests;
    let click_req = reqs
        .iter()
        .find(|r| matches!(r, ScannerRequest::Type(_)))
        .expect("No Type request");

    if let ScannerRequest::Type(cmd) = click_req {
        assert_eq!(cmd.id, 2);
    }
}
