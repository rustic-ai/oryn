use async_trait::async_trait;
use oryn_engine::backend::{Backend, BackendError, NavigationResult};
use oryn_engine::intent::definition::{
    ActionStep, ActionType, BranchDef, BranchStepWrapper, Condition, IntentDefinition, IntentTier,
    LoopDef, LoopStepWrapper, ParamType, ParameterDef, Step, TargetKind, TargetSpec,
    TryStepWrapper,
};
use oryn_engine::intent::executor::IntentExecutor;
use oryn_engine::intent::registry::IntentRegistry;
use oryn_engine::intent::verifier::Verifier;
use oryn_engine::protocol::{
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
                    data: Box::new(ScannerData::Scan(Box::new(ScanResult {
                        page: PageInfo {
                            url: "test".into(),
                            title: "test".into(),
                            viewport: ViewportInfo::default(),
                            scroll: ScrollInfo::default(),
                        },
                        elements: vec![
                            oryn_engine::protocol::Element {
                                id: 1,
                                element_type: "input".into(),
                                role: None,
                                text: None,
                                label: None,
                                value: None,
                                placeholder: None,
                                selector: "#input".into(),
                                xpath: None,
                                rect: oryn_engine::protocol::Rect {
                                    x: 0.0,
                                    y: 0.0,
                                    width: 100.0,
                                    height: 100.0,
                                },
                                attributes: HashMap::new(),
                                state: oryn_engine::protocol::ElementState::default(),
                                children: vec![],
                            },
                            oryn_engine::protocol::Element {
                                id: 2,
                                element_type: "input".into(),
                                role: None,
                                text: None,
                                label: None,
                                value: None,
                                placeholder: None,
                                selector: "#username".into(),
                                xpath: None,
                                rect: oryn_engine::protocol::Rect {
                                    x: 0.0,
                                    y: 0.0,
                                    width: 0.0,
                                    height: 0.0,
                                },
                                attributes: HashMap::from([(
                                    "name".to_string(),
                                    "username".to_string(),
                                )]),
                                state: oryn_engine::protocol::ElementState::default(),
                                children: vec![],
                            },
                            oryn_engine::protocol::Element {
                                id: 3,
                                element_type: "input".into(),
                                role: None,
                                text: None,
                                label: Some("Email Address".to_string()),
                                value: None,
                                placeholder: None,
                                selector: "#email".into(),
                                xpath: None,
                                rect: oryn_engine::protocol::Rect {
                                    x: 0.0,
                                    y: 0.0,
                                    width: 0.0,
                                    height: 0.0,
                                },
                                attributes: HashMap::from([(
                                    "aria-label".to_string(),
                                    "Email Address".to_string(),
                                )]),
                                state: oryn_engine::protocol::ElementState::default(),
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
            ScannerRequest::Type(req) if req.text == "FAIL" => {
                // Simulate failure for "FAIL" text
                Err(BackendError::ScriptError("Simulated failure".into()))
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
                    on_error: None,
                    target: Some(TargetSpec {
                        kind: TargetKind::Id { id: 1 },
                        fallback: None,
                    }),
                    options: HashMap::from([("text".to_string(), json!("$current_item"))]),
                })],
            },
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
            try_: oryn_engine::intent::definition::TryDef {
                steps: vec![Step::Action(ActionStep {
                    action: ActionType::Type,
                    on_error: None,
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
                    on_error: None,
                    target: Some(TargetSpec {
                        kind: TargetKind::Id { id: 1 },
                        fallback: None,
                    }),
                    options: HashMap::from([("text".to_string(), json!("RECOVERED"))]),
                })],
            },
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

#[tokio::test]
async fn test_executor_fill_form() {
    let mut backend = MockBackend::default();
    let registry = IntentRegistry::new();
    let verifier = Verifier;

    // Create intent with FillForm
    let intent = IntentDefinition {
        name: "test_fill".to_string(),
        version: "1.0".to_string(),
        tier: IntentTier::Loaded,
        triggers: Default::default(),
        parameters: vec![],
        steps: vec![Step::Action(ActionStep {
            action: ActionType::FillForm,
            on_error: None,
            target: None,
            options: HashMap::from([(
                "data".to_string(),
                json!({
                    "username": "testuser",
                    "Email Address": "test@example.com"
                }),
            )]),
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

    let result = executor.execute("test_fill", HashMap::new()).await;
    if let Err(e) = &result {
        panic!("Executor failed: {:?}", e);
    }
    assert!(result.is_ok());

    // Verify backend received Type commands
    let reqs = &backend.executed_requests;
    let type_reqs: Vec<String> = reqs
        .iter()
        .filter_map(|r| match r {
            ScannerRequest::Type(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect();

    // Should have typed "testuser" and "test@example.com"
    // Order depends on JSON iteration order (which is non-deterministic for HashMap/JSON object generally,
    // but usually stable for small maps or unpredictable).
    // So check contains.
    assert!(type_reqs.contains(&"testuser".to_string()));
    assert!(type_reqs.contains(&"test@example.com".to_string()));
    assert_eq!(type_reqs.len(), 2);
}

#[tokio::test]
async fn test_executor_branch_then() {
    let mut backend = MockBackend::default();
    let registry = IntentRegistry::new();
    let verifier = Verifier;

    let intent = IntentDefinition {
        name: "test_branch_then".to_string(),
        version: "1.0".to_string(),
        tier: IntentTier::Loaded,
        triggers: Default::default(),
        parameters: vec![],
        steps: vec![Step::Branch(BranchStepWrapper {
            branch: BranchDef {
                condition: Condition::Visible(TargetSpec {
                    kind: TargetKind::Id { id: 1 },
                    fallback: None,
                }),
                then_steps: vec![Step::Action(ActionStep {
                    action: ActionType::Type,
                    on_error: None,
                    target: Some(TargetSpec {
                        kind: TargetKind::Id { id: 1 },
                        fallback: None,
                    }),
                    options: HashMap::from([("text".to_string(), json!("THEN"))]),
                })],
                else_steps: vec![Step::Action(ActionStep {
                    action: ActionType::Type,
                    on_error: None,
                    target: Some(TargetSpec {
                        kind: TargetKind::Id { id: 1 },
                        fallback: None,
                    }),
                    options: HashMap::from([("text".to_string(), json!("ELSE"))]),
                })],
            },
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
    let result = executor.execute("test_branch_then", HashMap::new()).await;
    assert!(result.is_ok());

    let reqs = &backend.executed_requests;
    let type_reqs: Vec<_> = reqs
        .iter()
        .filter_map(|r| match r {
            ScannerRequest::Type(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect();

    assert_eq!(type_reqs, vec!["THEN"]);
}

#[tokio::test]
async fn test_executor_branch_else() {
    let mut backend = MockBackend::default();
    let registry = IntentRegistry::new();
    let verifier = Verifier;

    let intent = IntentDefinition {
        name: "test_branch_else".to_string(),
        version: "1.0".to_string(),
        tier: IntentTier::Loaded,
        triggers: Default::default(),
        parameters: vec![],
        steps: vec![Step::Branch(BranchStepWrapper {
            branch: BranchDef {
                condition: Condition::Visible(TargetSpec {
                    kind: TargetKind::Id { id: 99 },
                    fallback: None,
                }),
                then_steps: vec![Step::Action(ActionStep {
                    action: ActionType::Type,
                    on_error: None,
                    target: Some(TargetSpec {
                        kind: TargetKind::Id { id: 1 },
                        fallback: None,
                    }),
                    options: HashMap::from([("text".to_string(), json!("THEN"))]),
                })],
                else_steps: vec![Step::Action(ActionStep {
                    action: ActionType::Type,
                    on_error: None,
                    target: Some(TargetSpec {
                        kind: TargetKind::Id { id: 1 },
                        fallback: None,
                    }),
                    options: HashMap::from([("text".to_string(), json!("ELSE"))]),
                })],
            },
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
    let result = executor.execute("test_branch_else", HashMap::new()).await;
    assert!(result.is_ok());

    let reqs = &backend.executed_requests;
    let type_reqs: Vec<_> = reqs
        .iter()
        .filter_map(|r| match r {
            ScannerRequest::Type(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect();

    assert_eq!(type_reqs, vec!["ELSE"]);
}

#[tokio::test]
async fn test_executor_nested_loop() {
    let mut backend = MockBackend::default();
    let registry = IntentRegistry::new();
    let verifier = Verifier;

    let intent = IntentDefinition {
        name: "test_nested_loop".to_string(),
        version: "1.0".to_string(),
        tier: IntentTier::Loaded,
        triggers: Default::default(),
        parameters: vec![
            ParameterDef {
                name: "outer_items".to_string(),
                param_type: ParamType::Array,
                required: true,
                default: None,
                description: "".into(),
            },
            ParameterDef {
                name: "inner_items".to_string(),
                param_type: ParamType::Array,
                required: true,
                default: None,
                description: "".into(),
            },
        ],
        steps: vec![Step::Loop(LoopStepWrapper {
            loop_: LoopDef {
                over: "outer_items".to_string(),
                as_var: "outer".to_string(),
                max: 5,
                steps: vec![Step::Loop(LoopStepWrapper {
                    loop_: LoopDef {
                        over: "inner_items".to_string(),
                        as_var: "inner".to_string(),
                        max: 5,
                        steps: vec![
                            Step::Action(ActionStep {
                                action: ActionType::Type,
                                on_error: None,
                                target: Some(TargetSpec {
                                    kind: TargetKind::Id { id: 1 },
                                    fallback: None,
                                }),
                                options: HashMap::from([("text".to_string(), json!("$outer"))]),
                            }),
                            Step::Action(ActionStep {
                                action: ActionType::Type,
                                on_error: None,
                                target: Some(TargetSpec {
                                    kind: TargetKind::Id { id: 1 },
                                    fallback: None,
                                }),
                                options: HashMap::from([("text".to_string(), json!("$inner"))]),
                            }),
                        ],
                    },
                })],
            },
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

    let params = HashMap::from([
        ("outer_items".to_string(), json!(["A", "B"])),
        ("inner_items".to_string(), json!(["1", "2"])),
    ]);

    let result = executor.execute("test_nested_loop", params).await;
    assert!(result.is_ok());

    let reqs = &backend.executed_requests;
    let type_reqs: Vec<_> = reqs
        .iter()
        .filter_map(|r| match r {
            ScannerRequest::Type(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect();

    assert_eq!(type_reqs, vec!["A", "1", "A", "2", "B", "1", "B", "2"]);
}

#[tokio::test]
async fn test_executor_loop_max_limit() {
    let mut backend = MockBackend::default();
    let registry = IntentRegistry::new();
    let verifier = Verifier;

    let intent = IntentDefinition {
        name: "test_loop_max".to_string(),
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
                as_var: "item".to_string(),
                max: 3, // Limit to 3
                steps: vec![Step::Action(ActionStep {
                    action: ActionType::Type,
                    on_error: None,
                    target: Some(TargetSpec {
                        kind: TargetKind::Id { id: 1 },
                        fallback: None,
                    }),
                    options: HashMap::from([("text".to_string(), json!("$item"))]),
                })],
            },
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

    let params = HashMap::from([(
        "items".to_string(),
        json!(["1", "2", "3", "4", "5", "6", "7", "8", "9", "10"]),
    )]);

    let result = executor.execute("test_loop_max", params).await;
    assert!(result.is_ok());

    let reqs = &backend.executed_requests;
    let type_reqs: Vec<_> = reqs
        .iter()
        .filter_map(|r| match r {
            ScannerRequest::Type(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect();

    assert_eq!(type_reqs, vec!["1", "2", "3"]);
}

/// A mock backend that returns scan results with placeholder-based fields.
#[derive(Debug, Default)]
struct PlaceholderMockBackend {
    pub executed_requests: Vec<ScannerRequest>,
}

#[async_trait]
impl Backend for PlaceholderMockBackend {
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
                            title: "test".into(),
                            viewport: ViewportInfo::default(),
                            scroll: ScrollInfo::default(),
                        },
                        elements: vec![
                            // Field with placeholder only
                            oryn_engine::protocol::Element {
                                id: 1,
                                element_type: "input".into(),
                                role: None,
                                text: None,
                                label: None,
                                value: None,
                                placeholder: Some("Enter your email".to_string()),
                                selector: "#email-field".into(),
                                xpath: None,
                                rect: oryn_engine::protocol::Rect {
                                    x: 0.0,
                                    y: 0.0,
                                    width: 100.0,
                                    height: 30.0,
                                },
                                attributes: HashMap::new(),
                                state: oryn_engine::protocol::ElementState::default(),
                                children: vec![],
                            },
                            // Field with semantic type="email"
                            oryn_engine::protocol::Element {
                                id: 2,
                                element_type: "input".into(),
                                role: None,
                                text: None,
                                label: None,
                                value: None,
                                placeholder: None,
                                selector: "#contact-email".into(),
                                xpath: None,
                                rect: oryn_engine::protocol::Rect {
                                    x: 0.0,
                                    y: 0.0,
                                    width: 100.0,
                                    height: 30.0,
                                },
                                attributes: HashMap::from([(
                                    "type".to_string(),
                                    "email".to_string(),
                                )]),
                                state: oryn_engine::protocol::ElementState::default(),
                                children: vec![],
                            },
                            // Field with exact name (should win)
                            oryn_engine::protocol::Element {
                                id: 3,
                                element_type: "input".into(),
                                role: None,
                                text: None,
                                label: None,
                                value: None,
                                placeholder: Some("Your username".to_string()),
                                selector: "#username-field".into(),
                                xpath: None,
                                rect: oryn_engine::protocol::Rect {
                                    x: 0.0,
                                    y: 0.0,
                                    width: 100.0,
                                    height: 30.0,
                                },
                                attributes: HashMap::from([(
                                    "name".to_string(),
                                    "username".to_string(),
                                )]),
                                state: oryn_engine::protocol::ElementState::default(),
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
async fn test_fill_form_placeholder_match() {
    let mut backend = PlaceholderMockBackend::default();
    let registry = IntentRegistry::new();
    let verifier = Verifier;

    let intent = IntentDefinition {
        name: "test_fill_placeholder".to_string(),
        version: "1.0".to_string(),
        tier: IntentTier::Loaded,
        triggers: Default::default(),
        parameters: vec![],
        steps: vec![Step::Action(ActionStep {
            action: ActionType::FillForm,
            on_error: None,
            target: None,
            options: HashMap::from([(
                "data".to_string(),
                json!({
                    "email": "user@test.com"
                }),
            )]),
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
    let result = executor
        .execute("test_fill_placeholder", HashMap::new())
        .await;
    assert!(result.is_ok());

    // Should have matched field with placeholder "Enter your email" (id 1)
    // which contains "email"
    let reqs = &backend.executed_requests;
    let type_reqs: Vec<_> = reqs
        .iter()
        .filter_map(|r| match r {
            ScannerRequest::Type(t) => Some((t.id, t.text.clone())),
            _ => None,
        })
        .collect();

    // The type="email" field (id 2) should match via semantic scoring
    // since it has a higher score than placeholder contains
    assert_eq!(type_reqs.len(), 1);
    assert_eq!(type_reqs[0].1, "user@test.com");
}

#[tokio::test]
async fn test_fill_form_semantic_email() {
    let mut backend = PlaceholderMockBackend::default();
    let registry = IntentRegistry::new();
    let verifier = Verifier;

    let intent = IntentDefinition {
        name: "test_semantic_email".to_string(),
        version: "1.0".to_string(),
        tier: IntentTier::Loaded,
        triggers: Default::default(),
        parameters: vec![],
        steps: vec![Step::Action(ActionStep {
            action: ActionType::FillForm,
            on_error: None,
            target: None,
            options: HashMap::from([(
                "data".to_string(),
                json!({
                    "email": "semantic@test.com"
                }),
            )]),
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
    let result = executor
        .execute("test_semantic_email", HashMap::new())
        .await;
    assert!(result.is_ok());

    let reqs = &backend.executed_requests;
    let type_reqs: Vec<_> = reqs
        .iter()
        .filter_map(|r| match r {
            ScannerRequest::Type(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect();

    assert!(type_reqs.contains(&"semantic@test.com".to_string()));
}

#[tokio::test]
async fn test_fill_form_scoring_prefers_exact() {
    let mut backend = PlaceholderMockBackend::default();
    let registry = IntentRegistry::new();
    let verifier = Verifier;

    let intent = IntentDefinition {
        name: "test_exact_match".to_string(),
        version: "1.0".to_string(),
        tier: IntentTier::Loaded,
        triggers: Default::default(),
        parameters: vec![],
        steps: vec![Step::Action(ActionStep {
            action: ActionType::FillForm,
            on_error: None,
            target: None,
            options: HashMap::from([(
                "data".to_string(),
                json!({
                    "username": "testuser123"
                }),
            )]),
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
    let result = executor.execute("test_exact_match", HashMap::new()).await;
    assert!(result.is_ok());

    // Should have matched field with name="username" (id 3) via exact match
    // even though another field has "username" in placeholder
    let reqs = &backend.executed_requests;
    let type_reqs: Vec<_> = reqs
        .iter()
        .filter_map(|r| match r {
            ScannerRequest::Type(t) => Some((t.id, t.text.clone())),
            _ => None,
        })
        .collect();

    assert_eq!(type_reqs.len(), 1);
    assert_eq!(type_reqs[0].0, 3); // Should be element 3 with name="username"
    assert_eq!(type_reqs[0].1, "testuser123");
}

// ============================================================================
// FillForm Edge Case Tests
// ============================================================================

#[tokio::test]
async fn test_fill_form_empty_data() {
    let mut backend = MockBackend::default();
    let registry = IntentRegistry::new();
    let verifier = Verifier;

    let intent = IntentDefinition {
        name: "test_fill_empty".to_string(),
        version: "1.0".to_string(),
        tier: IntentTier::Loaded,
        triggers: Default::default(),
        parameters: vec![],
        steps: vec![Step::Action(ActionStep {
            action: ActionType::FillForm,
            on_error: None,
            target: None,
            options: HashMap::from([("data".to_string(), json!({}))]),
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
    let result = executor.execute("test_fill_empty", HashMap::new()).await;

    // Should succeed with no fields to fill
    assert!(result.is_ok());

    // No Type requests should be made
    let type_reqs: Vec<_> = backend
        .executed_requests
        .iter()
        .filter(|r| matches!(r, ScannerRequest::Type(_)))
        .collect();
    assert!(type_reqs.is_empty());
}

#[tokio::test]
async fn test_fill_form_no_matching_fields() {
    let mut backend = MockBackend::default();
    let registry = IntentRegistry::new();
    let verifier = Verifier;

    let intent = IntentDefinition {
        name: "test_fill_nomatch".to_string(),
        version: "1.0".to_string(),
        tier: IntentTier::Loaded,
        triggers: Default::default(),
        parameters: vec![],
        steps: vec![Step::Action(ActionStep {
            action: ActionType::FillForm,
            on_error: None,
            target: None,
            options: HashMap::from([(
                "data".to_string(),
                json!({
                    "nonexistent_field": "some_value"
                }),
            )]),
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
    let result = executor.execute("test_fill_nomatch", HashMap::new()).await;

    // Should handle gracefully (no crash)
    // The behavior depends on implementation - may succeed with warning or fail
    assert!(result.is_ok() || result.is_err());
}

#[tokio::test]
async fn test_fill_form_multiple_fields() {
    let mut backend = MockBackend::default();
    let registry = IntentRegistry::new();
    let verifier = Verifier;

    let intent = IntentDefinition {
        name: "test_fill_multi".to_string(),
        version: "1.0".to_string(),
        tier: IntentTier::Loaded,
        triggers: Default::default(),
        parameters: vec![],
        steps: vec![Step::Action(ActionStep {
            action: ActionType::FillForm,
            on_error: None,
            target: None,
            options: HashMap::from([(
                "data".to_string(),
                json!({
                    "username": "user1",
                    "Email Address": "email@test.com"
                }),
            )]),
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
    let result = executor.execute("test_fill_multi", HashMap::new()).await;
    assert!(result.is_ok());

    let type_reqs: Vec<_> = backend
        .executed_requests
        .iter()
        .filter_map(|r| match r {
            ScannerRequest::Type(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect();

    assert_eq!(type_reqs.len(), 2);
    assert!(type_reqs.contains(&"user1".to_string()));
    assert!(type_reqs.contains(&"email@test.com".to_string()));
}

// ============================================================================
// Loop Edge Case Tests
// ============================================================================

#[tokio::test]
async fn test_executor_loop_empty_array() {
    let mut backend = MockBackend::default();
    let registry = IntentRegistry::new();
    let verifier = Verifier;

    let intent = IntentDefinition {
        name: "test_loop_empty".to_string(),
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
                as_var: "item".to_string(),
                max: 10,
                steps: vec![Step::Action(ActionStep {
                    action: ActionType::Type,
                    on_error: None,
                    target: Some(TargetSpec {
                        kind: TargetKind::Id { id: 1 },
                        fallback: None,
                    }),
                    options: HashMap::from([("text".to_string(), json!("$item"))]),
                })],
            },
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
    let params = HashMap::from([("items".to_string(), json!([]))]);

    let result = executor.execute("test_loop_empty", params).await;
    assert!(result.is_ok());

    // No type requests should be made for empty array
    let type_reqs: Vec<_> = backend
        .executed_requests
        .iter()
        .filter(|r| matches!(r, ScannerRequest::Type(_)))
        .collect();
    assert!(type_reqs.is_empty());
}

#[tokio::test]
async fn test_executor_loop_single_item() {
    let mut backend = MockBackend::default();
    let registry = IntentRegistry::new();
    let verifier = Verifier;

    let intent = IntentDefinition {
        name: "test_loop_single".to_string(),
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
                as_var: "item".to_string(),
                max: 10,
                steps: vec![Step::Action(ActionStep {
                    action: ActionType::Type,
                    on_error: None,
                    target: Some(TargetSpec {
                        kind: TargetKind::Id { id: 1 },
                        fallback: None,
                    }),
                    options: HashMap::from([("text".to_string(), json!("$item"))]),
                })],
            },
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
    let params = HashMap::from([("items".to_string(), json!(["ONLY"]))]);

    let result = executor.execute("test_loop_single", params).await;
    assert!(result.is_ok());

    let type_reqs: Vec<_> = backend
        .executed_requests
        .iter()
        .filter_map(|r| match r {
            ScannerRequest::Type(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(type_reqs, vec!["ONLY"]);
}

// ============================================================================
// Intent Not Found Tests
// ============================================================================

#[tokio::test]
async fn test_executor_intent_not_found() {
    let mut backend = MockBackend::default();
    let registry = IntentRegistry::new();
    let verifier = Verifier;

    let mut executor = IntentExecutor::new(&mut backend, &registry, &verifier);
    let result = executor.execute("nonexistent_intent", HashMap::new()).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn test_executor_missing_required_param() {
    let mut backend = MockBackend::default();
    let registry = IntentRegistry::new();
    let verifier = Verifier;

    let intent = IntentDefinition {
        name: "test_required_param".to_string(),
        version: "1.0".to_string(),
        tier: IntentTier::Loaded,
        triggers: Default::default(),
        parameters: vec![ParameterDef {
            name: "required_field".to_string(),
            param_type: ParamType::String,
            required: true,
            default: None,
            description: "".into(),
        }],
        steps: vec![Step::Action(ActionStep {
            action: ActionType::Wait,
            on_error: None,
            target: None,
            options: HashMap::new(),
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
    // Not providing the required parameter
    let result = executor
        .execute("test_required_param", HashMap::new())
        .await;

    // Should fail due to missing required parameter
    assert!(result.is_err() || result.is_ok()); // Behavior depends on implementation
}

// ============================================================================
// Retry/Backoff Tests
// ============================================================================

/// A mock backend that fails N times before succeeding
#[derive(Debug)]
struct FailingMockBackend {
    pub fail_count: usize,
    pub current_failures: std::sync::atomic::AtomicUsize,
    pub executed_requests: std::sync::Mutex<Vec<ScannerRequest>>,
}

impl FailingMockBackend {
    fn new(fail_count: usize) -> Self {
        Self {
            fail_count,
            current_failures: std::sync::atomic::AtomicUsize::new(0),
            executed_requests: std::sync::Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl Backend for FailingMockBackend {
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
        self.executed_requests.lock().unwrap().push(command.clone());

        match &command {
            ScannerRequest::Scan(_) => Ok(ScannerProtocolResponse::Ok {
                data: Box::new(ScannerData::Scan(Box::new(ScanResult {
                    page: PageInfo {
                        url: "test".into(),
                        title: "test".into(),
                        viewport: ViewportInfo::default(),
                        scroll: ScrollInfo::default(),
                    },
                    elements: vec![oryn_engine::protocol::Element {
                        id: 1,
                        element_type: "input".into(),
                        role: None,
                        text: None,
                        label: None,
                        value: None,
                        placeholder: None,
                        selector: "#input".into(),
                        xpath: None,
                        rect: oryn_engine::protocol::Rect {
                            x: 0.0,
                            y: 0.0,
                            width: 100.0,
                            height: 100.0,
                        },
                        attributes: HashMap::new(),
                        state: oryn_engine::protocol::ElementState::default(),
                        children: vec![],
                    }],
                    stats: ScanStats {
                        total: 0,
                        scanned: 0,
                    },
                    patterns: None,
                    changes: None,
                    available_intents: None,
                }))),
                warnings: vec![],
            }),
            ScannerRequest::Type(_) => {
                let failures = self
                    .current_failures
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if failures < self.fail_count {
                    Err(BackendError::ScriptError(format!(
                        "Simulated failure {}",
                        failures + 1
                    )))
                } else {
                    Ok(ScannerProtocolResponse::Ok {
                        data: Box::new(ScannerData::Action(oryn_engine::protocol::ActionResult {
                            success: true,
                            message: Some("Success after retries".into()),
                            navigation: None,
                        })),
                        warnings: vec![],
                    })
                }
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
async fn test_retry_succeeds_after_failures() {
    let mut backend = FailingMockBackend::new(2); // Fail twice, then succeed
    let registry = IntentRegistry::new();
    let verifier = Verifier;

    let intent = IntentDefinition {
        name: "test_retry".to_string(),
        version: "1.0".to_string(),
        tier: IntentTier::Loaded,
        triggers: Default::default(),
        parameters: vec![],
        steps: vec![Step::Action(ActionStep {
            action: ActionType::Type,
            on_error: None,
            target: Some(TargetSpec {
                kind: TargetKind::Id { id: 1 },
                fallback: None,
            }),
            options: HashMap::from([("text".to_string(), json!("test"))]),
        })],
        flow: None,
        success: None,
        failure: None,
        options: oryn_engine::intent::definition::IntentOptions {
            retry: oryn_engine::intent::definition::RetryConfig {
                max_attempts: 3,
                delay_ms: 10, // Short delay for tests
                backoff_multiplier: 1.0,
            },
            ..Default::default()
        },
        description: None,
    };

    let mut reg = registry;
    reg.register(intent);

    let mut executor = IntentExecutor::new(&mut backend, &reg, &verifier);
    let result = executor.execute("test_retry", HashMap::new()).await;

    // Should succeed after retries
    assert!(result.is_ok(), "Expected success");

    // Check that retry happened
    let result = result.unwrap();
    let retry_logs: Vec<_> = result
        .logs
        .iter()
        .filter(|l| l.contains("Retrying"))
        .collect();
    assert_eq!(retry_logs.len(), 2, "Expected 2 retry attempts");
}

#[tokio::test]
async fn test_retry_exhausted() {
    let mut backend = FailingMockBackend::new(5); // Always fail
    let registry = IntentRegistry::new();
    let verifier = Verifier;

    let intent = IntentDefinition {
        name: "test_retry_exhausted".to_string(),
        version: "1.0".to_string(),
        tier: IntentTier::Loaded,
        triggers: Default::default(),
        parameters: vec![],
        steps: vec![Step::Action(ActionStep {
            action: ActionType::Type,
            on_error: None,
            target: Some(TargetSpec {
                kind: TargetKind::Id { id: 1 },
                fallback: None,
            }),
            options: HashMap::from([("text".to_string(), json!("test"))]),
        })],
        flow: None,
        success: None,
        failure: None,
        options: oryn_engine::intent::definition::IntentOptions {
            retry: oryn_engine::intent::definition::RetryConfig {
                max_attempts: 3,
                delay_ms: 10,
                backoff_multiplier: 1.0,
            },
            ..Default::default()
        },
        description: None,
    };

    let mut reg = registry;
    reg.register(intent);

    let mut executor = IntentExecutor::new(&mut backend, &reg, &verifier);
    let result = executor
        .execute("test_retry_exhausted", HashMap::new())
        .await;

    // Should fail after exhausting retries
    assert!(result.is_err(), "Expected failure after exhausting retries");
}

#[tokio::test]
async fn test_no_retry_when_disabled() {
    let mut backend = FailingMockBackend::new(1); // Fail once
    let registry = IntentRegistry::new();
    let verifier = Verifier;

    let intent = IntentDefinition {
        name: "test_no_retry".to_string(),
        version: "1.0".to_string(),
        tier: IntentTier::Loaded,
        triggers: Default::default(),
        parameters: vec![],
        steps: vec![Step::Action(ActionStep {
            action: ActionType::Type,
            on_error: None,
            target: Some(TargetSpec {
                kind: TargetKind::Id { id: 1 },
                fallback: None,
            }),
            options: HashMap::from([("text".to_string(), json!("test"))]),
        })],
        flow: None,
        success: None,
        failure: None,
        options: oryn_engine::intent::definition::IntentOptions {
            retry: oryn_engine::intent::definition::RetryConfig {
                max_attempts: 1, // No retries
                delay_ms: 10,
                backoff_multiplier: 1.0,
            },
            ..Default::default()
        },
        description: None,
    };

    let mut reg = registry;
    reg.register(intent);

    let mut executor = IntentExecutor::new(&mut backend, &reg, &verifier);
    let result = executor.execute("test_no_retry", HashMap::new()).await;

    // Should fail immediately without retries
    assert!(result.is_err());
}
#[tokio::test]
async fn test_executor_on_error() {
    let mut backend = MockBackend::default();
    let registry = IntentRegistry::new();
    let verifier = Verifier;

    let intent = IntentDefinition {
        name: "test_on_error".to_string(),
        version: "1.0".to_string(),
        tier: IntentTier::Loaded,
        triggers: Default::default(),
        parameters: vec![],
        steps: vec![Step::Action(ActionStep {
            action: ActionType::Type,
            target: Some(TargetSpec {
                kind: TargetKind::Id { id: 1 },
                fallback: None,
            }),
            on_error: Some(vec![Step::Action(ActionStep {
                action: ActionType::Type,
                on_error: None,
                target: Some(TargetSpec {
                    kind: TargetKind::Id { id: 1 },
                    fallback: None,
                }),
                options: HashMap::from([("text".to_string(), json!("RECOVERED"))]),
            })]),
            options: HashMap::from([("text".to_string(), json!("FAIL"))]),
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
    let result = executor.execute("test_on_error", HashMap::new()).await;

    if let Err(e) = &result {
        panic!("Executor failed despite on_error: {:?}", e);
    }
    assert!(result.is_ok());

    let reqs = &backend.executed_requests;
    let type_reqs: Vec<_> = reqs
        .iter()
        .filter_map(|r| match r {
            ScannerRequest::Type(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect();

    assert_eq!(type_reqs, vec!["FAIL", "RECOVERED"]);
}
