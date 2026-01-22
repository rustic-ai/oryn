use oryn_core::command::{Command, Target};
use oryn_core::formatter;
use oryn_core::protocol::{
    ActionResult, ClickRequest, Element, ElementState, MouseButton, PageInfo, Rect, ScanRequest,
    ScanResult, ScanStats, ScannerData, ScannerProtocolResponse, ScannerRequest, ScrollInfo,
    ViewportInfo,
};
use oryn_core::translator::{self, TranslationError};
use serde_json::json;
use std::collections::HashMap;

#[test]
fn test_protocol_serialization() {
    // Test ClickRequest serialization
    let req = ScannerRequest::Click(ClickRequest {
        id: 42,
        button: MouseButton::Left,
        double: false,
        modifiers: vec!["Alt".into()],
        force: false,
    });

    let json_str = serde_json::to_string(&req).unwrap();
    let expected = r#"{"action":"click","id":42,"button":"left","double":false,"modifiers":["Alt"],"force":false}"#;
    assert_eq!(json_str, expected);

    // Test ScanRequest serialization defaults
    let req = ScannerRequest::Scan(ScanRequest::default());
    let json_str = serde_json::to_string(&req).unwrap();
    let val: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(val["action"], "scan");
    assert_eq!(val["monitor_changes"], false);
}

#[test]
fn test_translator_click() {
    // Translate valid Click(Id)
    let cmd = Command::Click(Target::Id(123), HashMap::new());
    let ScannerRequest::Click(c) = translator::translate(&cmd).expect("Translation failed") else {
        panic!("Expected Click request");
    };
    assert_eq!(c.id, 123);
    assert!(c.modifiers.is_empty());

    // Translate invalid Click(Text) should return InvalidTarget error
    let cmd = Command::Click(Target::Text("Login".into()), HashMap::new());
    let Err(TranslationError::InvalidTarget(_)) = translator::translate(&cmd) else {
        panic!("Expected InvalidTarget error");
    };
}

#[test]
fn test_translator_type() {
    let cmd = Command::Type(Target::Id(55), "Hello".into(), HashMap::new());
    let ScannerRequest::Type(t) = translator::translate(&cmd).expect("Translation failed") else {
        panic!("Expected Type request");
    };
    assert_eq!(t.id, 55);
    assert_eq!(t.text, "Hello");
}

#[test]
fn test_formatter_ok() {
    let result = ScannerData::Action(ActionResult {
        success: true,
        message: Some("Clicked successfully".into()),
        navigation: None,
    });

    let resp = ScannerProtocolResponse::Ok {
        data: Box::new(result),
        warnings: vec![],
    };

    assert_eq!(
        formatter::format_response_with_intent(&resp, None),
        "Operation successful."
    );
}

#[test]
fn test_formatter_scan_scanresult() {
    let scan_res = ScanResult {
        page: PageInfo {
            url: "https://example.com".into(),
            title: "Example Domain".into(),
            viewport: ViewportInfo {
                width: 1920,
                height: 1080,
                scale: 1.0,
            },
            scroll: ScrollInfo {
                x: 0,
                y: 0,
                max_x: 0,
                max_y: 1000,
            },
        },
        elements: vec![Element {
            id: 1,
            element_type: "button".into(),
            role: Some("button".into()),
            text: Some("Submit".into()),
            label: None,
            value: None,
            placeholder: None,
            selector: "#submit".into(),
            xpath: None,
            rect: Rect {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 30.0,
            },
            attributes: HashMap::new(),
            state: ElementState::default(),
            children: vec![],
        }],
        stats: ScanStats {
            total: 1,
            scanned: 1,
        },
        patterns: None,
        changes: None,
        available_intents: None,
    };

    let resp = ScannerProtocolResponse::Ok {
        data: Box::new(ScannerData::Scan(Box::new(scan_res))),
        warnings: vec![],
    };

    let output = formatter::format_response_with_intent(&resp, None);
    assert!(output.contains("Scanned 1 elements."));
    assert!(output.contains("Title: Example Domain"));
    assert!(output.contains("URL: https://example.com"));
}

#[test]
fn test_formatter_error() {
    let resp = ScannerProtocolResponse::Error {
        code: "ELEMENT_NOT_FOUND".into(),
        message: "Element 999 not found".into(),
        details: Some(json!({ "id": 999 })),
        hint: None,
    };

    assert_eq!(
        formatter::format_response_with_intent(&resp, None),
        "Error: Element 999 not found"
    );
}

#[test]
fn test_formatter_error_with_hint() {
    let resp = ScannerProtocolResponse::Error {
        code: "ELEMENT_NOT_FOUND".into(),
        message: "Element 42 not found".into(),
        details: None,
        hint: Some("Run observe to refresh element map".into()),
    };

    assert_eq!(
        formatter::format_response_with_intent(&resp, None),
        "Error: Element 42 not found"
    );
}
