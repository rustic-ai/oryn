use oryn_core::formatter::format_intent_result;
use oryn_core::intent::executor::{IntentResult, IntentStatus};
use oryn_core::protocol::PageChanges;

#[test]
fn test_format_intent_result_success() {
    let result = IntentResult {
        status: IntentStatus::Success,
        data: None,
        logs: vec!["Step 1: Clicked button".to_string()],
        checkpoint: None,
        hints: vec![],
        changes: Some(PageChanges {
            url: Some("https://example.com/dashboard".to_string()),
            title: Some("Dashboard".to_string()),
            removed: vec![],
            added: vec!["#user-menu".to_string()],
        }),
    };

    let output = format_intent_result(&result, "login");
    println!("{}", output);

    assert!(output.contains("✅ Intent 'login' completed successfully."));
    assert!(output.contains("Changes:"));
    assert!(output.contains("🌐 URL: https://example.com/dashboard"));
    assert!(output.contains("📄 Title: Dashboard"));
    assert!(output.contains("➕ Added 1 elements"));
    assert!(output.contains("Logs:"));
    assert!(output.contains("Step 1: Clicked button"));
}

#[test]
fn test_format_intent_result_masking() {
    let result = IntentResult {
        status: IntentStatus::Success,
        data: None,
        logs: vec![
            r#"Type "password123" into password field"#.to_string(),
            r#"Value: "secret-token""#.to_string(),
        ],
        checkpoint: None,
        hints: vec![],
        changes: None,
    };

    let output = format_intent_result(&result, "login");
    println!("{}", output);

    assert!(!output.contains("password123"));
    assert!(output.contains("********"));
    assert!(!output.contains("secret-token"));
}

#[test]
fn test_format_intent_result_partial() {
    let result = IntentResult {
        status: IntentStatus::PartialSuccess {
            completed: 2,
            total: 5,
        },
        data: None,
        logs: vec![],
        checkpoint: Some("step_2".to_string()),
        hints: vec!["Check network connection".to_string()],
        changes: None,
    };

    let output = format_intent_result(&result, "sync");
    println!("{}", output);

    assert!(output.contains("⚠️ Intent 'sync' completed partially (2/5)"));
    assert!(output.contains("Last Checkpoint: step_2"));
    assert!(output.contains("Hints:"));
    assert!(output.contains("- Check network connection"));
}
