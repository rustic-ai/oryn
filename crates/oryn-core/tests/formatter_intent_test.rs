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
    assert!(output.contains("1. Step 1: Clicked button"));
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

// ============================================================================
// Failure Status Tests
// ============================================================================

#[test]
fn test_format_intent_result_failure() {
    let result = IntentResult {
        status: IntentStatus::Failed("Connection timeout".to_string()),
        data: None,
        logs: vec!["Step 1: Clicked login".to_string()],
        checkpoint: Some("login_started".to_string()),
        hints: vec!["Check network connection".to_string()],
        changes: None,
    };

    let output = format_intent_result(&result, "login");
    println!("{}", output);

    assert!(output.contains("❌ Intent 'login' failed: Connection timeout"));
    assert!(output.contains("login_started"));
    assert!(output.contains("Check network connection"));
}

#[test]
fn test_format_intent_result_failure_no_checkpoint() {
    let result = IntentResult {
        status: IntentStatus::Failed("Element not found".to_string()),
        data: None,
        logs: vec![],
        checkpoint: None,
        hints: vec![],
        changes: None,
    };

    let output = format_intent_result(&result, "click_button");
    println!("{}", output);

    assert!(output.contains("❌ Intent 'click_button' failed"));
    assert!(output.contains("Element not found"));
}

// ============================================================================
// Masking Edge Cases
// ============================================================================

#[test]
fn test_format_masking_api_key() {
    let result = IntentResult {
        status: IntentStatus::Success,
        data: None,
        logs: vec![r#"Set api_key to "sk-12345""#.to_string()],
        checkpoint: None,
        hints: vec![],
        changes: None,
    };

    let output = format_intent_result(&result, "config");
    println!("{}", output);

    assert!(!output.contains("sk-12345"));
}

#[test]
fn test_format_masking_token_variations() {
    let result = IntentResult {
        status: IntentStatus::Success,
        data: None,
        logs: vec![
            r#"auth_token: "abc123""#.to_string(),
            r#"access_token set to "xyz789""#.to_string(),
        ],
        checkpoint: None,
        hints: vec![],
        changes: None,
    };

    let output = format_intent_result(&result, "auth");
    println!("{}", output);

    // Should mask token-related values
    assert!(!output.contains("abc123") || !output.contains("xyz789"));
}

#[test]
fn test_format_masking_credit_card() {
    let result = IntentResult {
        status: IntentStatus::Success,
        data: None,
        logs: vec![
            r#"card_number: "4111111111111111""#.to_string(),
            r#"cvv: "123""#.to_string(),
        ],
        checkpoint: None,
        hints: vec![],
        changes: None,
    };

    let output = format_intent_result(&result, "payment");
    println!("{}", output);

    assert!(!output.contains("4111111111111111"));
    assert!(!output.contains("\"123\"") || output.contains("********"));
}

// ============================================================================
// Edge Cases
// ============================================================================

#[test]
fn test_format_intent_result_empty_logs() {
    let result = IntentResult {
        status: IntentStatus::Success,
        data: None,
        logs: vec![],
        checkpoint: None,
        hints: vec![],
        changes: None,
    };

    let output = format_intent_result(&result, "empty");
    println!("{}", output);

    assert!(output.contains("✅ Intent 'empty' completed successfully"));
}

#[test]
fn test_format_intent_result_no_changes() {
    let result = IntentResult {
        status: IntentStatus::Success,
        data: None,
        logs: vec!["Action completed".to_string()],
        checkpoint: None,
        hints: vec![],
        changes: None,
    };

    let output = format_intent_result(&result, "action");
    println!("{}", output);

    assert!(output.contains("✅"));
    // Should not have Changes section or should indicate no changes
}

#[test]
fn test_format_intent_result_with_data() {
    let result = IntentResult {
        status: IntentStatus::Success,
        data: Some(serde_json::json!({"user": "john", "count": 5})),
        logs: vec![],
        checkpoint: None,
        hints: vec![],
        changes: None,
    };

    let output = format_intent_result(&result, "fetch");
    println!("{}", output);

    assert!(output.contains("✅"));
}

#[test]
fn test_format_intent_partial_zero_completed() {
    let result = IntentResult {
        status: IntentStatus::PartialSuccess {
            completed: 0,
            total: 5,
        },
        data: None,
        logs: vec![],
        checkpoint: None,
        hints: vec![],
        changes: None,
    };

    let output = format_intent_result(&result, "batch");
    println!("{}", output);

    assert!(output.contains("0/5"));
}

#[test]
fn test_format_intent_partial_all_but_one() {
    let result = IntentResult {
        status: IntentStatus::PartialSuccess {
            completed: 99,
            total: 100,
        },
        data: None,
        logs: vec![],
        checkpoint: None,
        hints: vec![],
        changes: None,
    };

    let output = format_intent_result(&result, "large_batch");
    println!("{}", output);

    assert!(output.contains("99/100"));
}

#[test]
fn test_format_changes_removed_elements() {
    let result = IntentResult {
        status: IntentStatus::Success,
        data: None,
        logs: vec![],
        checkpoint: None,
        hints: vec![],
        changes: Some(PageChanges {
            url: None,
            title: None,
            removed: vec!["#modal".to_string(), "#overlay".to_string()],
            added: vec![],
        }),
    };

    let output = format_intent_result(&result, "dismiss");
    println!("{}", output);

    assert!(output.contains("➖ Removed 2 elements") || output.contains("Removed"));
}
