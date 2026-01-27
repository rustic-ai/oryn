use oryn_engine::formatter;
use oryn_engine::protocol::{
    ActionResult, ClickRequest, Element, ElementState, MouseButton, PageInfo, Rect, ScanRequest,
    ScanResult, ScanStats, ScannerAction, ScannerData, ScannerProtocolResponse, ScrollInfo,
    ViewportInfo,
};
use serde_json::json;
use std::collections::HashMap;

#[test]
fn test_protocol_serialization() {
    // Test ClickRequest serialization
    let req = ScannerAction::Click(ClickRequest {
        id: Some(42),
        selector: None,
        button: MouseButton::Left,
        double: false,
        modifiers: vec!["Alt".into()],
        force: false,
    });

    let json_str = serde_json::to_string(&req).unwrap();
    // Serialization depends on serde tag. ScannerAction uses tag="action".
    // ClickRequest fields are flattened? No, ScannerAction variants are usually structurally containing request.
    // Check protocol definition.
    // If ScannerAction::Click(ClickRequest), and #[serde(tag="action", rename_all="snake_case")],
    // then it serializes to `{"action":"click", "id": 42, ...}` due to flattening?
    // Usually enum variants with 1 field are not automatically flattened unless `#[serde(flatten)]` or `content` used?
    // `oryn-common/src/protocol.rs` defines `ScannerAction`:
    // #[derive(Serialize, Deserialize, Debug, Clone)]
    // #[serde(tag = "action", rename_all = "snake_case")]
    // pub enum ScannerAction {
    //    Scan(ScanRequest),
    //    Click(ClickRequest), ...
    // }
    // serde untagged flattening happens if the variant struct fields are compatible?
    // Wait, `tag="action"` means it adds `action` field.
    // The fields of `ClickRequest` are included in the object.

    let expected = r#"{"action":"click","id":42,"button":"left","double":false,"modifiers":["Alt"],"force":false}"#;
    assert_eq!(json_str, expected);

    // Test ScanRequest serialization defaults
    let req = ScannerAction::Scan(ScanRequest::default());
    let json_str = serde_json::to_string(&req).unwrap();
    let val: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert_eq!(val["action"], "scan");
    assert_eq!(val["monitor_changes"], false);
}

#[test]
fn test_formatter_ok() {
    let result = ScannerData::Action(ActionResult {
        success: true,
        message: Some("Clicked successfully".into()),
        navigation: None,
        dom_changes: None,
        value: None,
        coordinates: None,
    });

    let resp = ScannerProtocolResponse::Ok {
        data: Box::new(result),
        warnings: vec![],
    };

    assert_eq!(
        formatter::format_response(&resp),
        "Action Result: success=true, msg=Some(\"Clicked successfully\")"
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
        full_mode: false,
    };

    let resp = ScannerProtocolResponse::Ok {
        data: Box::new(ScannerData::Scan(Box::new(scan_res))),
        warnings: vec![],
    };

    let output = formatter::format_response(&resp);

    // Check for OIL format
    assert!(output.contains("@ https://example.com \"Example Domain\""));
    assert!(output.contains("[1] button/button \"Submit\""));
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
        formatter::format_response(&resp),
        "Error: Element 999 not found"
    );
}
