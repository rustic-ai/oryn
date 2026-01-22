use oryn_core::command::{Command, Target};
use oryn_core::protocol::{
    Element, ElementState, PageInfo, Rect, ScanResult, ScanStats, ScrollInfo, ViewportInfo,
};
use oryn_core::resolution::engine::ResolutionEngine;
use std::collections::HashMap;

/// Mock backend for testing
#[derive(Debug, Default)]
struct MockBackend;

#[async_trait::async_trait]
impl oryn_core::backend::Backend for MockBackend {
    async fn launch(&mut self) -> Result<(), oryn_core::backend::BackendError> {
        Ok(())
    }
    async fn close(&mut self) -> Result<(), oryn_core::backend::BackendError> {
        Ok(())
    }
    async fn is_ready(&self) -> bool {
        true
    }
    async fn navigate(
        &mut self,
        _url: &str,
    ) -> Result<oryn_core::backend::NavigationResult, oryn_core::backend::BackendError> {
        unimplemented!()
    }
    async fn go_back(
        &mut self,
    ) -> Result<oryn_core::backend::NavigationResult, oryn_core::backend::BackendError> {
        unimplemented!()
    }
    async fn go_forward(
        &mut self,
    ) -> Result<oryn_core::backend::NavigationResult, oryn_core::backend::BackendError> {
        unimplemented!()
    }
    async fn refresh(
        &mut self,
    ) -> Result<oryn_core::backend::NavigationResult, oryn_core::backend::BackendError> {
        unimplemented!()
    }
    async fn screenshot(&mut self) -> Result<Vec<u8>, oryn_core::backend::BackendError> {
        unimplemented!()
    }
    async fn pdf(&mut self) -> Result<Vec<u8>, oryn_core::backend::BackendError> {
        unimplemented!()
    }
    async fn get_cookies(
        &mut self,
    ) -> Result<Vec<oryn_core::protocol::Cookie>, oryn_core::backend::BackendError> {
        unimplemented!()
    }
    async fn set_cookie(
        &mut self,
        _cookie: oryn_core::protocol::Cookie,
    ) -> Result<(), oryn_core::backend::BackendError> {
        unimplemented!()
    }
    async fn get_tabs(
        &mut self,
    ) -> Result<Vec<oryn_core::protocol::TabInfo>, oryn_core::backend::BackendError> {
        unimplemented!()
    }
    async fn press_key(
        &mut self,
        _key: &str,
        _modifiers: &[String],
    ) -> Result<(), oryn_core::backend::BackendError> {
        unimplemented!()
    }
    async fn execute_scanner(
        &mut self,
        _command: oryn_core::protocol::ScannerRequest,
    ) -> Result<oryn_core::protocol::ScannerProtocolResponse, oryn_core::backend::BackendError>
    {
        unimplemented!()
    }
}

fn make_element(id: u32, type_: &str, text: Option<&str>) -> Element {
    Element {
        id,
        element_type: type_.to_string(),
        role: None,
        text: text.map(|s| s.to_string()),
        label: None,
        value: None,
        placeholder: None,
        selector: format!("#{}", id),
        xpath: None,
        rect: Rect {
            x: 0.0,
            y: (id * 40) as f32,
            width: 200.0,
            height: 30.0,
        },
        attributes: HashMap::new(),
        state: ElementState::default(),
        children: vec![],
    }
}

fn make_scan(elements: Vec<Element>) -> ScanResult {
    ScanResult {
        page: PageInfo {
            url: "http://test.com".into(),
            title: "Test".into(),
            viewport: ViewportInfo::default(),
            scroll: ScrollInfo::default(),
        },
        elements,
        stats: ScanStats {
            total: 0,
            scanned: 0,
        },
        patterns: None,
        changes: None,
        available_intents: None,
    }
}

/// Test: `<label for="email">Email</label><input id="email">` -> type "Email" should resolve to input
#[tokio::test]
async fn test_for_attribute_association() {
    let mut backend = MockBackend::default();

    // Create label with for="email"
    let mut label = make_element(1, "label", Some("Email"));
    label
        .attributes
        .insert("for".to_string(), "email".to_string());
    label.rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 100.0,
        height: 30.0,
    };

    // Create input with id="email"
    let mut input = make_element(2, "input", None);
    input
        .attributes
        .insert("id".to_string(), "email".to_string());
    input
        .attributes
        .insert("type".to_string(), "text".to_string());
    input.rect = Rect {
        x: 110.0,
        y: 0.0,
        width: 200.0,
        height: 30.0,
    };

    let scan = make_scan(vec![label, input]);

    // type "Email" should resolve to input id=2
    let cmd = Command::Type(
        Target::Text("Email".to_string()),
        "test@example.com".to_string(),
        HashMap::new(),
    );
    let result = ResolutionEngine::resolve(cmd, &scan, &mut backend).await;

    assert!(result.is_ok(), "Expected Ok, got {:?}", result);
    match result.unwrap() {
        Command::Type(Target::Id(id), text, _) => {
            assert_eq!(id, 2, "Should resolve to input element");
            assert_eq!(text, "test@example.com");
        }
        other => panic!("Expected Type command with Id target, got {:?}", other),
    }
}

/// Test: `<label>Email <input></label>` -> type "Email" should resolve to nested input
#[tokio::test]
async fn test_nested_control_association() {
    let mut backend = MockBackend::default();

    // Create label containing an input
    let mut label = make_element(1, "label", Some("Email"));
    label.rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 300.0,
        height: 50.0,
    };

    // Create input nested inside label (bounding box inside label)
    let mut input = make_element(2, "input", None);
    input
        .attributes
        .insert("type".to_string(), "text".to_string());
    input.rect = Rect {
        x: 60.0,
        y: 10.0,
        width: 200.0,
        height: 30.0,
    };

    let scan = make_scan(vec![label, input]);

    // type "Email" should resolve to input id=2
    let cmd = Command::Type(
        Target::Text("Email".to_string()),
        "test@example.com".to_string(),
        HashMap::new(),
    );
    let result = ResolutionEngine::resolve(cmd, &scan, &mut backend).await;

    assert!(result.is_ok(), "Expected Ok, got {:?}", result);
    match result.unwrap() {
        Command::Type(Target::Id(id), _, _) => {
            assert_eq!(id, 2, "Should resolve to nested input element");
        }
        other => panic!("Expected Type command with Id target, got {:?}", other),
    }
}

/// Test: `<span>State</span><select>` -> select "State" should resolve to select element
#[tokio::test]
async fn test_adjacent_control_association() {
    let mut backend = MockBackend::default();

    // Create span (label-like) next to select
    let mut span = make_element(1, "span", Some("State"));
    span.rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 50.0,
        height: 30.0,
    };

    // Create select adjacent to span
    let mut select = make_element(2, "select", None);
    select.rect = Rect {
        x: 60.0,
        y: 0.0,
        width: 150.0,
        height: 30.0,
    };

    let scan = make_scan(vec![span, select]);

    // select "State" should resolve to select id=2
    let cmd = Command::Select(Target::Text("State".to_string()), "California".to_string());
    let result = ResolutionEngine::resolve(cmd, &scan, &mut backend).await;

    assert!(result.is_ok(), "Expected Ok, got {:?}", result);
    match result.unwrap() {
        Command::Select(Target::Id(id), value) => {
            assert_eq!(id, 2, "Should resolve to select element");
            assert_eq!(value, "California");
        }
        other => panic!("Expected Select command with Id target, got {:?}", other),
    }
}

/// Test: `<label for="remember">Remember me</label><input type="checkbox" id="remember">`
#[tokio::test]
async fn test_checkbox_association() {
    let mut backend = MockBackend::default();

    // Create label with for="remember"
    let mut label = make_element(1, "label", Some("Remember me"));
    label
        .attributes
        .insert("for".to_string(), "remember".to_string());
    label.rect = Rect {
        x: 30.0,
        y: 0.0,
        width: 100.0,
        height: 30.0,
    };

    // Create checkbox with id="remember"
    let mut checkbox = make_element(2, "input", None);
    checkbox
        .attributes
        .insert("id".to_string(), "remember".to_string());
    checkbox
        .attributes
        .insert("type".to_string(), "checkbox".to_string());
    checkbox.rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 20.0,
        height: 20.0,
    };

    let scan = make_scan(vec![label, checkbox]);

    // check "Remember me" should resolve to checkbox id=2
    let cmd = Command::Check(Target::Text("Remember me".to_string()));
    let result = ResolutionEngine::resolve(cmd, &scan, &mut backend).await;

    assert!(result.is_ok(), "Expected Ok, got {:?}", result);
    match result.unwrap() {
        Command::Check(Target::Id(id)) => {
            assert_eq!(id, 2, "Should resolve to checkbox element");
        }
        other => panic!("Expected Check command with Id target, got {:?}", other),
    }
}

/// Test: click "Email" on label should be allowed (browser handles focus/toggle)
#[tokio::test]
async fn test_label_click_allowed() {
    let mut backend = MockBackend::default();

    // Create label with for="email"
    let mut label = make_element(1, "label", Some("Email"));
    label
        .attributes
        .insert("for".to_string(), "email".to_string());
    label.rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 100.0,
        height: 30.0,
    };

    // Create input with id="email"
    let mut input = make_element(2, "input", None);
    input
        .attributes
        .insert("id".to_string(), "email".to_string());
    input
        .attributes
        .insert("type".to_string(), "text".to_string());
    input.rect = Rect {
        x: 110.0,
        y: 0.0,
        width: 200.0,
        height: 30.0,
    };

    let scan = make_scan(vec![label, input]);

    // click "Email" should resolve to the label (browser handles focus)
    let cmd = Command::Click(Target::Text("Email".to_string()), HashMap::new());
    let result = ResolutionEngine::resolve(cmd, &scan, &mut backend).await;

    assert!(result.is_ok(), "Expected Ok, got {:?}", result);
    match result.unwrap() {
        Command::Click(Target::Id(id), _) => {
            assert_eq!(
                id, 1,
                "Click should resolve to label element (browser handles focus)"
            );
        }
        other => panic!("Expected Click command with Id target, got {:?}", other),
    }
}

/// Test: orphan label (no associated control) should give helpful error for type command
#[tokio::test]
async fn test_no_association_error() {
    let mut backend = MockBackend::default();

    // Create orphan label with no associated input
    let mut label = make_element(1, "label", Some("Orphan Label"));
    label.rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 100.0,
        height: 30.0,
    };

    let scan = make_scan(vec![label]);

    // type "Orphan Label" should fail with helpful error
    let cmd = Command::Type(
        Target::Text("Orphan Label".to_string()),
        "test".to_string(),
        HashMap::new(),
    );
    let result = ResolutionEngine::resolve(cmd, &scan, &mut backend).await;

    assert!(result.is_err(), "Expected error for orphan label");
    let err = result.unwrap_err();
    assert!(
        err.reason.contains("no associated control"),
        "Error message should mention no associated control: {}",
        err.reason
    );
}

/// Test: requirement validation only triggers for Typeable/Selectable/Checkable
#[tokio::test]
async fn test_direct_input_resolution() {
    let mut backend = MockBackend::default();

    // Create input directly with label
    let mut input = make_element(1, "input", None);
    input.label = Some("Email".to_string());
    input
        .attributes
        .insert("type".to_string(), "text".to_string());
    input.rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 200.0,
        height: 30.0,
    };

    let scan = make_scan(vec![input]);

    // type "Email" should resolve directly to input (no association needed)
    let cmd = Command::Type(
        Target::Text("Email".to_string()),
        "test@example.com".to_string(),
        HashMap::new(),
    );
    let result = ResolutionEngine::resolve(cmd, &scan, &mut backend).await;

    assert!(result.is_ok(), "Expected Ok, got {:?}", result);
    match result.unwrap() {
        Command::Type(Target::Id(id), _, _) => {
            assert_eq!(id, 1, "Should resolve directly to input element");
        }
        other => panic!("Expected Type command with Id target, got {:?}", other),
    }
}

/// Test: Adjacent control with vertical proximity
#[tokio::test]
async fn test_adjacent_control_below_label() {
    let mut backend = MockBackend::default();

    // Create label above input
    let mut label = make_element(1, "label", Some("Description"));
    label.rect = Rect {
        x: 0.0,
        y: 0.0,
        width: 100.0,
        height: 30.0,
    };

    // Create textarea below label
    let mut textarea = make_element(2, "textarea", None);
    textarea.rect = Rect {
        x: 0.0,
        y: 35.0,
        width: 300.0,
        height: 100.0,
    };

    let scan = make_scan(vec![label, textarea]);

    // type "Description" should resolve to textarea
    let cmd = Command::Type(
        Target::Text("Description".to_string()),
        "Some text".to_string(),
        HashMap::new(),
    );
    let result = ResolutionEngine::resolve(cmd, &scan, &mut backend).await;

    assert!(result.is_ok(), "Expected Ok, got {:?}", result);
    match result.unwrap() {
        Command::Type(Target::Id(id), _, _) => {
            assert_eq!(id, 2, "Should resolve to textarea element below label");
        }
        other => panic!("Expected Type command with Id target, got {:?}", other),
    }
}
