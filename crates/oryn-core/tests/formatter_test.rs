use oryn_core::formatter::{format_response, format_response_with_intent};
use oryn_core::intent::definition::{IntentDefinition, IntentTier};
use oryn_core::intent::registry::IntentRegistry;
use oryn_core::protocol::{
    DetectedPatterns, LoginPattern, PageInfo, ScanResult, ScanStats, ScannerData,
    ScannerProtocolResponse, ScrollInfo, ViewportInfo,
};

fn mock_scan_result_with_login() -> ScannerProtocolResponse {
    let patterns = DetectedPatterns {
        login: Some(LoginPattern {
            email: None,
            username: None,
            password: 0,
            submit: None,
            remember: None,
        }),
        ..Default::default()
    };

    let scan = ScanResult {
        page: PageInfo {
            url: "https://example.com".into(),
            title: "Test".into(),
            viewport: ViewportInfo::default(),
            scroll: ScrollInfo::default(),
        },
        elements: vec![],
        stats: ScanStats {
            total: 0,
            scanned: 0,
        },
        patterns: Some(patterns),
        changes: None,
    };

    ScannerProtocolResponse::Ok {
        data: Box::new(ScannerData::Scan(scan)),
        warnings: vec![],
    }
}

fn mock_intent(name: &str) -> IntentDefinition {
    let mut intent = IntentDefinition {
        name: name.to_string(),
        version: "1.0.0".to_string(),
        tier: IntentTier::BuiltIn,
        triggers: Default::default(),
        parameters: vec![],
        steps: vec![],
        success: None,
        failure: None,
        options: Default::default(),
    };
    // Map login pattern to this intent
    intent.triggers.patterns.push("login_form".to_string());
    intent
}

#[test]
fn test_format_response_basic() {
    let resp = mock_scan_result_with_login();
    let output = format_response(&resp);
    assert!(output.contains("Patterns:"));
    assert!(output.contains("- Login Form"));
    assert!(!output.contains("Available Intents:"));
}

#[test]
fn test_format_response_with_intent() {
    let resp = mock_scan_result_with_login();
    let mut registry = IntentRegistry::new();
    registry.register(mock_intent("builtin_login"));

    let output = format_response_with_intent(&resp, Some(&registry));
    assert!(output.contains("Available Intents:"));
    assert!(output.contains("- builtin_login (v1.0.0)"));
}
