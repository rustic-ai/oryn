//! Protocol serialization and deserialization tests.
//!
//! These tests ensure that the ScannerData enum deserializes correctly,
//! particularly after removing the redundant ScanValidation variant.

use oryn_engine::protocol::{
    ActionResult, Element, ElementState, PageInfo, Rect, ScanResult, ScanStats, ScannerData,
    ScannerProtocolResponse, ScrollInfo, ViewportInfo,
};
use std::collections::HashMap;

/// Test that ScanResult deserializes into ScannerData::Scan variant
#[test]
fn test_scan_data_deserializes_correctly() {
    let json = r#"{
        "page": {
            "url": "https://example.com",
            "title": "Example",
            "viewport": {"width": 1920, "height": 1080, "scale": 1.0},
            "scroll": {"x": 0, "y": 0, "max_x": 0, "max_y": 1000}
        },
        "elements": [],
        "stats": {"total": 0, "scanned": 0}
    }"#;

    let data: ScannerData = serde_json::from_str(json).unwrap();
    assert!(
        matches!(data, ScannerData::Scan(_)),
        "Expected ScannerData::Scan variant"
    );

    if let ScannerData::Scan(scan) = data {
        assert_eq!(scan.page.url, "https://example.com");
        assert_eq!(scan.page.title, "Example");
        assert!(scan.elements.is_empty());
    }
}

/// Test that ActionResult deserializes into ScannerData::Action variant
#[test]
fn test_action_data_deserializes_correctly() {
    let json = r#"{
        "success": true,
        "message": "clicked"
    }"#;

    let data: ScannerData = serde_json::from_str(json).unwrap();
    assert!(
        matches!(data, ScannerData::Action(_)),
        "Expected ScannerData::Action variant"
    );

    if let ScannerData::Action(action) = data {
        assert!(action.success);
        assert_eq!(action.message.as_deref(), Some("clicked"));
    }
}

/// Test that ActionResult with navigation field deserializes correctly
#[test]
fn test_action_data_with_navigation_deserializes() {
    let json = r#"{
        "success": true,
        "message": "navigated",
        "navigation": true
    }"#;

    let data: ScannerData = serde_json::from_str(json).unwrap();
    assert!(matches!(data, ScannerData::Action(_)));

    if let ScannerData::Action(action) = data {
        assert!(action.success);
        assert_eq!(action.navigation, Some(true));
    }
}

/// Test that Value variant catches arbitrary JSON
#[test]
fn test_value_data_deserializes_correctly() {
    // This should NOT match Scan (missing required fields) or Action (missing success)
    let json = r#"{"arbitrary": "data", "count": 42}"#;

    let data: ScannerData = serde_json::from_str(json).unwrap();
    assert!(
        matches!(data, ScannerData::Value(_)),
        "Expected ScannerData::Value variant"
    );
}

/// Test full ScannerProtocolResponse::Ok with Scan data
#[test]
fn test_protocol_response_ok_scan() {
    let json = r##"{
        "status": "ok",
        "page": {
            "url": "https://test.com",
            "title": "Test Page",
            "viewport": {"width": 800, "height": 600, "scale": 1.0},
            "scroll": {"x": 0, "y": 100, "max_x": 0, "max_y": 2000}
        },
        "elements": [
            {
                "id": 1,
                "type": "button",
                "text": "Submit",
                "selector": "#submit-btn",
                "rect": {"x": 10, "y": 20, "width": 100, "height": 30}
            }
        ],
        "stats": {"total": 10, "scanned": 1},
        "warnings": []
    }"##;

    let resp: ScannerProtocolResponse = serde_json::from_str(json).unwrap();
    assert!(matches!(resp, ScannerProtocolResponse::Ok { .. }));

    if let ScannerProtocolResponse::Ok { data, warnings } = resp {
        assert!(warnings.is_empty());
        assert!(matches!(data.as_ref(), ScannerData::Scan(_)));

        if let ScannerData::Scan(scan) = data.as_ref() {
            assert_eq!(scan.elements.len(), 1);
            assert_eq!(scan.elements[0].id, 1);
            assert_eq!(scan.elements[0].text.as_deref(), Some("Submit"));
        }
    }
}

/// Test ScannerProtocolResponse::Ok with Action data
#[test]
fn test_protocol_response_ok_action() {
    let json = r#"{
        "status": "ok",
        "success": true,
        "message": "Element clicked"
    }"#;

    let resp: ScannerProtocolResponse = serde_json::from_str(json).unwrap();
    assert!(matches!(resp, ScannerProtocolResponse::Ok { .. }));

    if let ScannerProtocolResponse::Ok { data, .. } = resp {
        assert!(matches!(data.as_ref(), ScannerData::Action(_)));
    }
}

/// Test ScannerProtocolResponse::Error
#[test]
fn test_protocol_response_error() {
    let json = r#"{
        "status": "error",
        "code": "ELEMENT_NOT_FOUND",
        "message": "Element 42 not found"
    }"#;

    let resp: ScannerProtocolResponse = serde_json::from_str(json).unwrap();
    assert!(matches!(resp, ScannerProtocolResponse::Error { .. }));

    if let ScannerProtocolResponse::Error { code, message, .. } = resp {
        assert_eq!(code, "ELEMENT_NOT_FOUND");
        assert_eq!(message, "Element 42 not found");
    }
}

/// Test ScannerProtocolResponse::Error with hint
#[test]
fn test_protocol_response_error_with_hint() {
    let json = r#"{
        "status": "error",
        "code": "ELEMENT_STALE",
        "message": "Element 5 is stale",
        "hint": "Run observe to refresh element map"
    }"#;

    let resp: ScannerProtocolResponse = serde_json::from_str(json).unwrap();
    if let ScannerProtocolResponse::Error { hint, .. } = resp {
        assert_eq!(hint.as_deref(), Some("Run observe to refresh element map"));
    }
}

/// Test scan result with patterns
#[test]
fn test_scan_result_with_patterns() {
    let json = r#"{
        "page": {
            "url": "https://login.example.com",
            "title": "Login",
            "viewport": {"width": 1920, "height": 1080, "scale": 1.0},
            "scroll": {"x": 0, "y": 0, "max_x": 0, "max_y": 0}
        },
        "elements": [],
        "stats": {"total": 5, "scanned": 5},
        "patterns": {
            "login": {
                "password": 3,
                "submit": 4
            }
        }
    }"#;

    let data: ScannerData = serde_json::from_str(json).unwrap();
    if let ScannerData::Scan(scan) = data {
        assert!(scan.patterns.is_some());
        let patterns = scan.patterns.unwrap();
        assert!(patterns.login.is_some());
        let login = patterns.login.unwrap();
        assert_eq!(login.password, 3);
        assert_eq!(login.submit, Some(4));
    } else {
        panic!("Expected ScannerData::Scan");
    }
}

/// Test scan result with available intents
#[test]
fn test_scan_result_with_available_intents() {
    let json = r#"{
        "page": {
            "url": "https://example.com",
            "title": "Example",
            "viewport": {"width": 1920, "height": 1080, "scale": 1.0},
            "scroll": {"x": 0, "y": 0, "max_x": 0, "max_y": 0}
        },
        "elements": [],
        "stats": {"total": 0, "scanned": 0},
        "available_intents": [
            {
                "name": "login",
                "status": "ready",
                "parameters": ["username", "password"]
            },
            {
                "name": "search",
                "status": "navigate_required",
                "parameters": ["query"]
            }
        ]
    }"#;

    let data: ScannerData = serde_json::from_str(json).unwrap();
    if let ScannerData::Scan(scan) = data {
        assert!(scan.available_intents.is_some());
        let intents = scan.available_intents.unwrap();
        assert_eq!(intents.len(), 2);
        assert_eq!(intents[0].name, "login");
        assert_eq!(
            intents[0].status,
            oryn_engine::protocol::AvailabilityStatus::Ready
        );
        assert_eq!(intents[0].parameters, vec!["username", "password"]);
    } else {
        panic!("Expected ScannerData::Scan");
    }
}

/// Test element with full state
#[test]
fn test_element_deserialization_full() {
    let json = r##"{
        "id": 42,
        "type": "input",
        "role": "textbox",
        "text": null,
        "label": "Email Address",
        "value": "test@example.com",
        "placeholder": "Enter your email",
        "selector": "#email",
        "xpath": "//input[@id='email']",
        "rect": {"x": 100, "y": 200, "width": 300, "height": 40},
        "attributes": {
            "name": "email",
            "required": "true"
        },
        "state": {
            "checked": false,
            "selected": false,
            "disabled": false,
            "readonly": false,
            "expanded": false,
            "focused": true
        },
        "children": [1, 2, 3]
    }"##;

    let element: Element = serde_json::from_str(json).unwrap();
    assert_eq!(element.id, 42);
    assert_eq!(element.element_type, "input");
    assert_eq!(element.role.as_deref(), Some("textbox"));
    assert_eq!(element.label.as_deref(), Some("Email Address"));
    assert_eq!(element.value.as_deref(), Some("test@example.com"));
    assert_eq!(element.placeholder.as_deref(), Some("Enter your email"));
    assert!(element.state.focused);
    assert!(!element.state.disabled);
    assert_eq!(element.children, vec![1, 2, 3]);
}

/// Test ScanResult roundtrip (serialize then deserialize)
#[test]
fn test_scan_result_roundtrip() {
    let original = ScanResult {
        page: PageInfo {
            url: "https://roundtrip.test".to_string(),
            title: "Roundtrip Test".to_string(),
            viewport: ViewportInfo {
                width: 1920,
                height: 1080,
                scale: 2.0,
            },
            scroll: ScrollInfo {
                x: 100,
                y: 200,
                max_x: 500,
                max_y: 3000,
            },
        },
        elements: vec![Element {
            id: 1,
            element_type: "button".to_string(),
            role: Some("button".to_string()),
            text: Some("Click Me".to_string()),
            label: None,
            value: None,
            placeholder: None,
            selector: "button.primary".to_string(),
            xpath: None,
            rect: Rect {
                x: 10.0,
                y: 20.0,
                width: 100.0,
                height: 40.0,
            },
            attributes: HashMap::from([("class".to_string(), "primary".to_string())]),
            state: ElementState::default(),
            children: vec![],
        }],
        stats: ScanStats {
            total: 100,
            scanned: 50,
        },
        patterns: None,
        changes: None,
        available_intents: None,
    };

    let json = serde_json::to_string(&original).unwrap();
    let deserialized: ScanResult = serde_json::from_str(&json).unwrap();

    assert_eq!(deserialized.page.url, original.page.url);
    assert_eq!(deserialized.page.title, original.page.title);
    assert_eq!(deserialized.elements.len(), original.elements.len());
    assert_eq!(deserialized.elements[0].id, original.elements[0].id);
    assert_eq!(deserialized.stats.total, original.stats.total);
}

/// Test ActionResult roundtrip
#[test]
fn test_action_result_roundtrip() {
    let original = ActionResult {
        success: true,
        message: Some("Action completed".to_string()),
        navigation: Some(false),
    };

    let json = serde_json::to_string(&original).unwrap();
    let deserialized: ActionResult = serde_json::from_str(&json).unwrap();

    assert_eq!(deserialized.success, original.success);
    assert_eq!(deserialized.message, original.message);
    assert_eq!(deserialized.navigation, original.navigation);
}
