use oryn_core::command::{Command, Target};
use oryn_core::formatter;
use oryn_core::protocol::{
    Element, ElementState, PageInfo, Rect, ScanResult, ScanStats, ScannerData,
    ScannerProtocolResponse, ScannerRequest, ScrollInfo, ViewportInfo,
};
use oryn_core::translator;
use serde_json::json;
use std::collections::HashMap;

#[test]
fn test_protocol_serialization() {
    // Test ClickRequest serialization
    let req = ScannerRequest::Click(oryn_core::protocol::ClickRequest {
        id: 42,
        button: oryn_core::protocol::MouseButton::Left,
        double: false,
        modifiers: vec!["Alt".to_string()],
    });

    let json = serde_json::to_string(&req).unwrap();
    let expected =
        r#"{"action":"click","id":42,"button":"left","double":false,"modifiers":["Alt"]}"#;
    assert_eq!(json, expected);

    // Test ScanRequest serialization defaults
    let req = ScannerRequest::Scan(oryn_core::protocol::ScanRequest::default());
    let json = serde_json::to_string(&req).unwrap();
    // Verify it contains the tag and fields
    let val: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(val["action"], "scan");
    assert_eq!(val["monitor_changes"], false);
}

#[test]
fn test_translator_click() {
    // Translate valid Click(Id)
    let cmd = Command::Click(Target::Id(123), HashMap::new());
    let req = translator::translate(&cmd).expect("Translation failed");

    match req {
        ScannerRequest::Click(c) => {
            assert_eq!(c.id, 123);
            assert_eq!(c.modifiers.len(), 0);
        }
        _ => panic!("Wrong request type"),
    }

    // Translate invalid Click(Text)
    let cmd = Command::Click(Target::Text("Login".into()), HashMap::new());
    let err = translator::translate(&cmd);
    assert!(err.is_err());
    match err {
        Err(oryn_core::translator::TranslationError::InvalidTarget(_)) => {}
        _ => panic!("Wrong error type"),
    }
}

#[test]
fn test_translator_type() {
    let cmd = Command::Type(Target::Id(55), "Hello".into(), HashMap::new());
    let req = translator::translate(&cmd).expect("Translation failed");

    match req {
        ScannerRequest::Type(t) => {
            assert_eq!(t.id, 55);
            assert_eq!(t.text, "Hello");
        }
        _ => panic!("Wrong request type"),
    }
}

#[test]
fn test_formatter_ok() {
    let result = ScannerData::Action(oryn_core::protocol::ActionResult {
        success: true,
        message: Some("Clicked successfully".into()),
        navigation: None,
    });

    let resp = ScannerProtocolResponse::Ok {
        data: Box::new(result),
        warnings: vec![],
    };

    let output = formatter::format_response(&resp);
    assert_eq!(output, "OK Clicked successfully");
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
    };

    let resp = ScannerProtocolResponse::Ok {
        data: Box::new(ScannerData::Scan(scan_res)),
        warnings: vec![],
    };

    let output = formatter::format_response(&resp);

    assert!(output.contains("@ https://example.com \"Example Domain\""));
    assert!(output.contains("[1] button/button \"Submit\""));
}

#[test]
fn test_formatter_error() {
    let resp = ScannerProtocolResponse::Error {
        code: "ELEMENT_NOT_FOUND".into(),
        message: "Element 999 not found".into(),
        details: Some(json!({ "id": 999 })),
    };

    let output = formatter::format_response(&resp);
    assert_eq!(
        output,
        "ERROR [ELEMENT_NOT_FOUND]: Element 999 not found (Some(Object {\"id\": Number(999)}))"
    );
}
