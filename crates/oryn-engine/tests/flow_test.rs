use async_trait::async_trait;
use oryn_engine::backend::{Backend, BackendError, NavigationResult};
use oryn_engine::intent::definition::{
    ActionStep, ActionType, FlowDefinition, IntentDefinition, IntentOptions, IntentTier,
    PageAction, PageDef, PageTransition, Step, TargetKind, TargetSpec,
};
use oryn_engine::intent::executor::IntentExecutor;
use oryn_engine::intent::registry::IntentRegistry;
use oryn_engine::intent::schema::Validatable;
use oryn_engine::intent::verifier::Verifier;
use oryn_engine::protocol::{
    PageInfo, ScanResult, ScanStats, ScannerData, ScannerProtocolResponse, ScannerRequest,
    ScrollInfo, ViewportInfo,
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// =============================================================================
// Mock Backend for Flow Tests
// =============================================================================

/// A mock backend that simulates page navigation with URL changes
#[derive(Debug)]
struct FlowMockBackend {
    pub current_url: Arc<Mutex<String>>,
    pub executed_requests: Arc<Mutex<Vec<ScannerRequest>>>,
    pub navigation_history: Arc<Mutex<Vec<String>>>,
}

impl Default for FlowMockBackend {
    fn default() -> Self {
        Self {
            current_url: Arc::new(Mutex::new("https://example.com/cart".to_string())),
            executed_requests: Arc::new(Mutex::new(Vec::new())),
            navigation_history: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl FlowMockBackend {
    fn with_url(url: &str) -> Self {
        Self {
            current_url: Arc::new(Mutex::new(url.to_string())),
            executed_requests: Arc::new(Mutex::new(Vec::new())),
            navigation_history: Arc::new(Mutex::new(vec![url.to_string()])),
        }
    }
}

#[async_trait]
impl Backend for FlowMockBackend {
    async fn launch(&mut self) -> Result<(), BackendError> {
        Ok(())
    }

    async fn close(&mut self) -> Result<(), BackendError> {
        Ok(())
    }

    async fn is_ready(&self) -> bool {
        true
    }

    async fn navigate(&mut self, url: &str) -> Result<NavigationResult, BackendError> {
        *self.current_url.lock().unwrap() = url.to_string();
        self.navigation_history
            .lock()
            .unwrap()
            .push(url.to_string());
        Ok(NavigationResult {
            url: url.to_string(),
            title: "Test Page".to_string(),
            status: 200,
        })
    }

    async fn go_back(&mut self) -> Result<NavigationResult, BackendError> {
        let mut history = self.navigation_history.lock().unwrap();
        if history.len() > 1 {
            history.pop();
            let prev_url = history.last().cloned().unwrap_or_default();
            *self.current_url.lock().unwrap() = prev_url.clone();
            Ok(NavigationResult {
                url: prev_url,
                title: "Test Page".to_string(),
                status: 200,
            })
        } else {
            Err(BackendError::Navigation(
                "No history to go back to".to_string(),
            ))
        }
    }

    async fn go_forward(&mut self) -> Result<NavigationResult, BackendError> {
        Err(BackendError::NotSupported("go_forward".to_string()))
    }

    async fn refresh(&mut self) -> Result<NavigationResult, BackendError> {
        let url = self.current_url.lock().unwrap().clone();
        Ok(NavigationResult {
            url,
            title: "Test Page".to_string(),
            status: 200,
        })
    }

    async fn execute_scanner(
        &mut self,
        command: ScannerRequest,
    ) -> Result<ScannerProtocolResponse, BackendError> {
        self.executed_requests.lock().unwrap().push(command.clone());

        match &command {
            ScannerRequest::Scan(_) => {
                let url = self.current_url.lock().unwrap().clone();
                Ok(ScannerProtocolResponse::Ok {
                    data: Box::new(ScannerData::Scan(Box::new(ScanResult {
                        page: PageInfo {
                            url,
                            title: "Test Page".to_string(),
                            viewport: ViewportInfo::default(),
                            scroll: ScrollInfo::default(),
                        },
                        elements: vec![
                            oryn_engine::protocol::Element {
                                id: 1,
                                element_type: "button".to_string(),
                                role: None,
                                text: Some("Checkout".to_string()),
                                label: None,
                                value: None,
                                placeholder: None,
                                selector: "#checkout-btn".to_string(),
                                xpath: None,
                                rect: oryn_engine::protocol::Rect {
                                    x: 0.0,
                                    y: 0.0,
                                    width: 100.0,
                                    height: 40.0,
                                },
                                attributes: HashMap::new(),
                                state: oryn_engine::protocol::ElementState::default(),
                                children: vec![],
                            },
                            oryn_engine::protocol::Element {
                                id: 2,
                                element_type: "span".to_string(),
                                role: None,
                                text: Some("ORD-12345".to_string()),
                                label: None,
                                value: None,
                                placeholder: None,
                                selector: "#order-id".to_string(),
                                xpath: None,
                                rect: oryn_engine::protocol::Rect {
                                    x: 0.0,
                                    y: 0.0,
                                    width: 100.0,
                                    height: 20.0,
                                },
                                attributes: HashMap::new(),
                                state: oryn_engine::protocol::ElementState::default(),
                                children: vec![],
                            },
                        ],
                        stats: ScanStats {
                            total: 2,
                            scanned: 2,
                        },
                        patterns: None,
                        changes: None,
                        available_intents: None,
                    }))),
                    warnings: vec![],
                })
            }
            ScannerRequest::Click(req) => {
                // Simulate navigation on checkout button click
                if req.id == 1 {
                    let mut url = self.current_url.lock().unwrap();
                    if url.contains("/cart") {
                        *url = "https://example.com/checkout/shipping".to_string();
                    } else if url.contains("/shipping") {
                        *url = "https://example.com/checkout/payment".to_string();
                    } else if url.contains("/payment") {
                        *url = "https://example.com/confirmation".to_string();
                    }
                }
                Ok(ScannerProtocolResponse::Ok {
                    data: Box::new(ScannerData::Action(oryn_engine::protocol::ActionResult {
                        success: true,
                        message: Some("Clicked".to_string()),
                        navigation: None,
                    })),
                    warnings: vec![],
                })
            }
            _ => Ok(ScannerProtocolResponse::Ok {
                data: Box::new(ScannerData::Action(oryn_engine::protocol::ActionResult {
                    success: true,
                    message: Some("Action executed".to_string()),
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

// =============================================================================
// FlowDefinition Parsing Tests
// =============================================================================

#[test]
fn test_flow_definition_yaml_parsing() {
    let yaml = r##"
name: checkout_flow
version: "1.0.0"
tier: loaded

parameters:
  - name: shipping
    type: object
    required: true

flow:
  pages:
    - name: cart
      url_pattern: ".*/cart.*"
      intents:
        - verify_cart
      next:
        page: shipping

    - name: shipping
      url_pattern: ".*/checkout/shipping.*"
      intents:
        - fill_form
      next:
        page: payment

    - name: payment
      url_pattern: ".*/checkout/payment.*"
      next:
        page: confirmation

    - name: confirmation
      url_pattern: ".*/confirmation.*"
      extract:
        order_number:
          selector: "#order-id"
      next: end
"##;

    let intent: IntentDefinition = serde_yaml::from_str(yaml).expect("Failed to parse YAML");

    assert_eq!(intent.name, "checkout_flow");
    assert!(intent.flow.is_some());

    let flow = intent.flow.as_ref().unwrap();
    assert_eq!(flow.pages.len(), 4);
    assert_eq!(flow.pages[0].name, "cart");
    assert_eq!(flow.pages[1].name, "shipping");
    assert_eq!(flow.pages[2].name, "payment");
    assert_eq!(flow.pages[3].name, "confirmation");

    // Check transitions
    assert!(matches!(
        flow.pages[0].next,
        Some(PageTransition::Page { page: ref s }) if s == "shipping"
    ));
    assert!(matches!(flow.pages[3].next, Some(PageTransition::End(_))));
}

#[test]
fn test_flow_with_inline_steps() {
    let yaml = r##"
name: inline_steps_flow
version: "1.0.0"
tier: loaded

flow:
  pages:
    - name: login
      url_pattern: ".*/login.*"
      intents:
        - steps:
            - action: type
              target:
                selector: "#username"
              text: "testuser"
            - action: click
              target:
                selector: "#submit"
      next: end
"##;

    let intent: IntentDefinition = serde_yaml::from_str(yaml).expect("Failed to parse YAML");

    assert!(intent.flow.is_some());
    let flow = intent.flow.as_ref().unwrap();
    assert_eq!(flow.pages.len(), 1);

    if let PageAction::Inline { steps } = &flow.pages[0].intents[0] {
        assert_eq!(steps.len(), 2);
    } else {
        panic!("Expected inline steps");
    }
}

#[test]
fn test_flow_with_start_page() {
    let yaml = r##"
name: flow_with_start
version: "1.0.0"
tier: loaded

flow:
  start: middle
  pages:
    - name: first
      url_pattern: ".*/first"
      next: end
    - name: middle
      url_pattern: ".*/middle"
      next:
        page: first
"##;

    let intent: IntentDefinition = serde_yaml::from_str(yaml).expect("Failed to parse YAML");

    let flow = intent.flow.as_ref().unwrap();
    assert_eq!(flow.start, Some("middle".to_string()));
}

// =============================================================================
// Schema Validation Tests
// =============================================================================

#[test]
fn test_flow_validation_duplicate_page_names() {
    let intent = IntentDefinition {
        name: "test".to_string(),
        description: None,
        version: "1.0.0".to_string(),
        tier: IntentTier::Loaded,
        triggers: Default::default(),
        parameters: vec![],
        steps: vec![],
        flow: Some(FlowDefinition {
            start: None,
            pages: vec![
                PageDef {
                    name: "page1".to_string(),
                    url_pattern: ".*".to_string(),
                    intents: vec![],
                    next: Some(PageTransition::end()),
                    on_error: None,
                    extract: None,
                },
                PageDef {
                    name: "page1".to_string(), // Duplicate!
                    url_pattern: ".*".to_string(),
                    intents: vec![],
                    next: Some(PageTransition::end()),
                    on_error: None,
                    extract: None,
                },
            ],
        }),
        success: None,
        failure: None,
        options: IntentOptions::default(),
    };

    let result = intent.validate();
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Duplicate page name")
    );
}

#[test]
fn test_flow_validation_invalid_transition() {
    let intent = IntentDefinition {
        name: "test".to_string(),
        description: None,
        version: "1.0.0".to_string(),
        tier: IntentTier::Loaded,
        triggers: Default::default(),
        parameters: vec![],
        steps: vec![],
        flow: Some(FlowDefinition {
            start: None,
            pages: vec![PageDef {
                name: "page1".to_string(),
                url_pattern: ".*".to_string(),
                intents: vec![],
                next: Some(PageTransition::to_page("nonexistent")),
                on_error: None,
                extract: None,
            }],
        }),
        success: None,
        failure: None,
        options: IntentOptions::default(),
    };

    let result = intent.validate();
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Invalid page transition")
    );
}

#[test]
fn test_flow_validation_invalid_start_page() {
    let intent = IntentDefinition {
        name: "test".to_string(),
        description: None,
        version: "1.0.0".to_string(),
        tier: IntentTier::Loaded,
        triggers: Default::default(),
        parameters: vec![],
        steps: vec![],
        flow: Some(FlowDefinition {
            start: Some("nonexistent".to_string()),
            pages: vec![PageDef {
                name: "page1".to_string(),
                url_pattern: ".*".to_string(),
                intents: vec![],
                next: Some(PageTransition::end()),
                on_error: None,
                extract: None,
            }],
        }),
        success: None,
        failure: None,
        options: IntentOptions::default(),
    };

    let result = intent.validate();
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Invalid start page")
    );
}

#[test]
fn test_flow_validation_steps_and_flow_mutually_exclusive() {
    let intent = IntentDefinition {
        name: "test".to_string(),
        description: None,
        version: "1.0.0".to_string(),
        tier: IntentTier::Loaded,
        triggers: Default::default(),
        parameters: vec![],
        steps: vec![Step::Action(ActionStep {
            action: ActionType::Click,
            on_error: None,
            target: None,
            options: HashMap::new(),
        })],
        flow: Some(FlowDefinition {
            start: None,
            pages: vec![PageDef {
                name: "page1".to_string(),
                url_pattern: ".*".to_string(),
                intents: vec![],
                next: Some(PageTransition::end()),
                on_error: None,
                extract: None,
            }],
        }),
        success: None,
        failure: None,
        options: IntentOptions::default(),
    };

    let result = intent.validate();
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("mutually exclusive")
    );
}

#[test]
fn test_flow_validation_empty_flow() {
    let intent = IntentDefinition {
        name: "test".to_string(),
        description: None,
        version: "1.0.0".to_string(),
        tier: IntentTier::Loaded,
        triggers: Default::default(),
        parameters: vec![],
        steps: vec![],
        flow: Some(FlowDefinition {
            start: None,
            pages: vec![], // Empty!
        }),
        success: None,
        failure: None,
        options: IntentOptions::default(),
    };

    let result = intent.validate();
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("at least one page")
    );
}

#[test]
fn test_flow_validation_valid_flow() {
    let intent = IntentDefinition {
        name: "valid_flow".to_string(),
        description: None,
        version: "1.0.0".to_string(),
        tier: IntentTier::Loaded,
        triggers: Default::default(),
        parameters: vec![],
        steps: vec![],
        flow: Some(FlowDefinition {
            start: Some("cart".to_string()),
            pages: vec![
                PageDef {
                    name: "cart".to_string(),
                    url_pattern: ".*/cart".to_string(),
                    intents: vec![],
                    next: Some(PageTransition::to_page("checkout")),
                    on_error: None,
                    extract: None,
                },
                PageDef {
                    name: "checkout".to_string(),
                    url_pattern: ".*/checkout".to_string(),
                    intents: vec![],
                    next: Some(PageTransition::end()),
                    on_error: None,
                    extract: None,
                },
            ],
        }),
        success: None,
        failure: None,
        options: IntentOptions::default(),
    };

    let result = intent.validate();
    assert!(result.is_ok(), "Expected valid flow: {:?}", result);
}

// =============================================================================
// Flow Execution Tests
// =============================================================================

#[tokio::test]
async fn test_flow_execution_simple() {
    let mut backend = FlowMockBackend::with_url("https://example.com/cart");
    let registry = IntentRegistry::new();
    let verifier = Verifier;

    // Create a simple flow intent
    let intent = IntentDefinition {
        name: "simple_flow".to_string(),
        description: None,
        version: "1.0.0".to_string(),
        tier: IntentTier::Loaded,
        triggers: Default::default(),
        parameters: vec![],
        steps: vec![],
        flow: Some(FlowDefinition {
            start: None,
            pages: vec![
                PageDef {
                    name: "cart".to_string(),
                    url_pattern: ".*/cart.*".to_string(),
                    intents: vec![PageAction::Inline {
                        steps: vec![Step::Action(ActionStep {
                            action: ActionType::Click,
                            on_error: None,
                            target: Some(TargetSpec {
                                kind: TargetKind::Id { id: 1 },
                                fallback: None,
                            }),
                            options: HashMap::new(),
                        })],
                    }],
                    next: Some(PageTransition::to_page("shipping")),
                    on_error: None,
                    extract: None,
                },
                PageDef {
                    name: "shipping".to_string(),
                    url_pattern: ".*/checkout/shipping.*".to_string(),
                    intents: vec![PageAction::Inline {
                        steps: vec![Step::Action(ActionStep {
                            action: ActionType::Click,
                            on_error: None,
                            target: Some(TargetSpec {
                                kind: TargetKind::Id { id: 1 },
                                fallback: None,
                            }),
                            options: HashMap::new(),
                        })],
                    }],
                    next: Some(PageTransition::to_page("payment")),
                    on_error: None,
                    extract: None,
                },
                PageDef {
                    name: "payment".to_string(),
                    url_pattern: ".*/checkout/payment.*".to_string(),
                    intents: vec![PageAction::Inline {
                        steps: vec![Step::Action(ActionStep {
                            action: ActionType::Click,
                            on_error: None,
                            target: Some(TargetSpec {
                                kind: TargetKind::Id { id: 1 },
                                fallback: None,
                            }),
                            options: HashMap::new(),
                        })],
                    }],
                    next: Some(PageTransition::to_page("confirmation")),
                    on_error: None,
                    extract: None,
                },
                PageDef {
                    name: "confirmation".to_string(),
                    url_pattern: ".*/confirmation.*".to_string(),
                    intents: vec![],
                    next: Some(PageTransition::end()),
                    on_error: None,
                    extract: None,
                },
            ],
        }),
        success: None,
        failure: None,
        options: IntentOptions {
            timeout: 5000,
            ..Default::default()
        },
    };

    let mut reg = registry;
    reg.register(intent);

    let mut executor = IntentExecutor::new(&mut backend, &reg, &verifier);
    let result = executor.execute("simple_flow", HashMap::new()).await;

    assert!(result.is_ok(), "Flow execution failed: {:?}", result.err());

    let result = result.unwrap();
    assert!(
        matches!(
            result.status,
            oryn_engine::intent::executor::IntentStatus::Success
        ),
        "Expected Success status, got {:?}",
        result.status
    );

    // Verify we went through all pages
    assert!(result.logs.iter().any(|l| l.contains("page 'cart'")));
    assert!(result.logs.iter().any(|l| l.contains("page 'shipping'")));
    assert!(result.logs.iter().any(|l| l.contains("page 'payment'")));
    assert!(
        result
            .logs
            .iter()
            .any(|l| l.contains("page 'confirmation'"))
    );
}

#[tokio::test]
async fn test_flow_data_extraction() {
    let mut backend = FlowMockBackend::with_url("https://example.com/confirmation");
    let registry = IntentRegistry::new();
    let verifier = Verifier;

    let intent = IntentDefinition {
        name: "extract_flow".to_string(),
        description: None,
        version: "1.0.0".to_string(),
        tier: IntentTier::Loaded,
        triggers: Default::default(),
        parameters: vec![],
        steps: vec![],
        flow: Some(FlowDefinition {
            start: None,
            pages: vec![PageDef {
                name: "confirmation".to_string(),
                url_pattern: ".*/confirmation.*".to_string(),
                intents: vec![],
                next: Some(PageTransition::end()),
                on_error: None,
                extract: Some(HashMap::from([(
                    "order_number".to_string(),
                    json!({"selector": "#order-id"}),
                )])),
            }],
        }),
        success: None,
        failure: None,
        options: IntentOptions {
            timeout: 5000,
            ..Default::default()
        },
    };

    let mut reg = registry;
    reg.register(intent);

    let mut executor = IntentExecutor::new(&mut backend, &reg, &verifier);
    let result = executor.execute("extract_flow", HashMap::new()).await;

    assert!(result.is_ok(), "Flow execution failed: {:?}", result.err());

    let result = result.unwrap();
    assert!(result.data.is_some(), "Expected extracted data");

    if let Some(data) = result.data {
        assert!(
            data.get("order_number").is_some(),
            "Expected order_number in extracted data: {:?}",
            data
        );
    }
}

// =============================================================================
// Navigation Action Tests
// =============================================================================

#[tokio::test]
async fn test_navigate_action() {
    let mut backend = FlowMockBackend::default();
    let registry = IntentRegistry::new();
    let verifier = Verifier;

    let intent = IntentDefinition {
        name: "nav_test".to_string(),
        description: None,
        version: "1.0.0".to_string(),
        tier: IntentTier::Loaded,
        triggers: Default::default(),
        parameters: vec![],
        steps: vec![Step::Action(ActionStep {
            action: ActionType::Navigate,
            on_error: None,
            target: None,
            options: HashMap::from([("url".to_string(), json!("https://example.com/new-page"))]),
        })],
        flow: None,
        success: None,
        failure: None,
        options: IntentOptions::default(),
    };

    let mut reg = registry;
    reg.register(intent);

    let mut executor = IntentExecutor::new(&mut backend, &reg, &verifier);
    let result = executor.execute("nav_test", HashMap::new()).await;

    assert!(result.is_ok(), "Navigate action failed: {:?}", result.err());

    let url = backend.current_url.lock().unwrap().clone();
    assert_eq!(url, "https://example.com/new-page");
}

#[tokio::test]
async fn test_go_back_action() {
    let mut backend = FlowMockBackend::default();
    // Add some history
    backend
        .navigation_history
        .lock()
        .unwrap()
        .push("https://example.com/page1".to_string());
    *backend.current_url.lock().unwrap() = "https://example.com/page1".to_string();
    backend
        .navigation_history
        .lock()
        .unwrap()
        .push("https://example.com/page2".to_string());
    *backend.current_url.lock().unwrap() = "https://example.com/page2".to_string();

    let registry = IntentRegistry::new();
    let verifier = Verifier;

    let intent = IntentDefinition {
        name: "back_test".to_string(),
        description: None,
        version: "1.0.0".to_string(),
        tier: IntentTier::Loaded,
        triggers: Default::default(),
        parameters: vec![],
        steps: vec![Step::Action(ActionStep {
            action: ActionType::GoBack,
            on_error: None,
            target: None,
            options: HashMap::new(),
        })],
        flow: None,
        success: None,
        failure: None,
        options: IntentOptions::default(),
    };

    let mut reg = registry;
    reg.register(intent);

    let mut executor = IntentExecutor::new(&mut backend, &reg, &verifier);
    let result = executor.execute("back_test", HashMap::new()).await;

    assert!(result.is_ok(), "GoBack action failed: {:?}", result.err());

    let url = backend.current_url.lock().unwrap().clone();
    assert_eq!(url, "https://example.com/page1");
}

#[tokio::test]
async fn test_refresh_action() {
    let mut backend = FlowMockBackend::with_url("https://example.com/current");
    let registry = IntentRegistry::new();
    let verifier = Verifier;

    let intent = IntentDefinition {
        name: "refresh_test".to_string(),
        description: None,
        version: "1.0.0".to_string(),
        tier: IntentTier::Loaded,
        triggers: Default::default(),
        parameters: vec![],
        steps: vec![Step::Action(ActionStep {
            action: ActionType::Refresh,
            on_error: None,
            target: None,
            options: HashMap::new(),
        })],
        flow: None,
        success: None,
        failure: None,
        options: IntentOptions::default(),
    };

    let mut reg = registry;
    reg.register(intent);

    let mut executor = IntentExecutor::new(&mut backend, &reg, &verifier);
    let result = executor.execute("refresh_test", HashMap::new()).await;

    assert!(result.is_ok(), "Refresh action failed: {:?}", result.err());

    // URL should remain the same
    let url = backend.current_url.lock().unwrap().clone();
    assert_eq!(url, "https://example.com/current");
}
