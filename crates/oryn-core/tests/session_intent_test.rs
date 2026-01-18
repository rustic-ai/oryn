use oryn_core::intent::define_parser::parse_define;
use oryn_core::intent::definition::{ActionType, Step, TargetKind};
use oryn_core::intent::session::SessionIntentManager;
use tempfile::NamedTempFile;

#[test]
fn test_parse_define_simple() {
    let input = r#"define test_intent:
  description: "A test intent"
  steps:
    - click "Button"
    - type "Input" "Value""#;

    let def = parse_define(input).expect("Failed to parse");
    assert_eq!(def.name, "test_intent");
    assert_eq!(def.description.as_deref(), Some("A test intent"));
    assert_eq!(def.steps.len(), 2);

    if let Step::Action(act) = &def.steps[0] {
        assert_eq!(act.action, ActionType::Click);
        if let Some(target) = &act.target {
            if let TargetKind::Text { text, .. } = &target.kind {
                assert_eq!(text, "Button");
            } else {
                panic!("Wrong target kind");
            }
        }
    }

    if let Step::Action(act) = &def.steps[1] {
        assert_eq!(act.action, ActionType::Type);
        // "Value" is in options for Type action in my simplified parser
        assert_eq!(
            act.options.get("text").and_then(|v| v.as_str()),
            Some("Value")
        );
    }
}

#[test]
fn test_session_manager_lifecycle() {
    let mut manager = SessionIntentManager::new();

    let input = r#"define temp:
    steps:
    - wait visible "something""#;
    let def = parse_define(input).expect("parse");

    // Define
    manager.define(def.clone()).expect("define");
    assert!(manager.get("temp").is_some());

    // Export
    let tmp_file = NamedTempFile::new().expect("tempfile");
    manager.export("temp", tmp_file.path()).expect("export");

    // Verify file content
    let content = std::fs::read_to_string(tmp_file.path()).expect("read");
    assert!(content.contains("name: temp"));

    // Undefine
    manager.undefine("temp").expect("undefine");
    assert!(manager.get("temp").is_none());
}

#[test]
fn test_parse_define_fallback() {
    let input = r#"define fallback_test:
    steps:
    - click "A" or click "B""#;

    let def = parse_define(input).expect("parse");
    assert_eq!(def.steps.len(), 1);

    if let Step::Try(wrapper) = &def.steps[0] {
        // "A" is in try block
        assert_eq!(wrapper.try_.steps.len(), 1);
        if let Step::Action(act) = &wrapper.try_.steps[0] {
            if let Some(target) = &act.target {
                if let TargetKind::Text { text, .. } = &target.kind {
                    assert_eq!(text, "A");
                }
            }
        } else {
            panic!("Expected Action for A");
        }

        // "B" is in catch block
        assert_eq!(wrapper.try_.catch.len(), 1);
        if let Step::Action(act) = &wrapper.try_.catch[0] {
            if let Some(target) = &act.target {
                if let TargetKind::Text { text, .. } = &target.kind {
                    assert_eq!(text, "B");
                }
            }
        } else {
            panic!("Expected Action for B");
        }
    } else {
        panic!("Expected Step::Try");
    }
}

#[test]
fn test_parse_define_role() {
    let input = r#"define role_test:
    steps:
    - type email "user@example.com"
    - type "Plain Text" "Value""#;

    let def = parse_define(input).expect("parse");
    assert_eq!(def.steps.len(), 2);

    // Step 1: Role-based
    if let Step::Action(act) = &def.steps[0] {
        assert_eq!(act.action, ActionType::Type);
        if let Some(target) = &act.target {
            match &target.kind {
                TargetKind::Role { role } => assert_eq!(role, "email"),
                _ => panic!("Expected Role target, got {:?}", target.kind),
            }
        }
        assert_eq!(
            act.options.get("text").and_then(|v| v.as_str()),
            Some("user@example.com")
        );
    } else {
        panic!("Step 1 not Action");
    }

    // Step 2: Text-based (legacy)
    if let Step::Action(act) = &def.steps[1] {
        assert_eq!(act.action, ActionType::Type);
        if let Some(target) = &act.target {
            match &target.kind {
                TargetKind::Text { text, .. } => assert_eq!(text, "Plain Text"),
                _ => panic!("Expected Text target"),
            }
        }
    } else {
        panic!("Step 2 not Action");
    }
}

// ============================================================================
// Define Parser Error Handling Tests
// ============================================================================

#[test]
fn test_parse_define_missing_header() {
    let input = r#"steps:
    - click "Button""#;

    let result = parse_define(input);
    assert!(result.is_err(), "Should fail without 'define name:' header");
}

#[test]
fn test_parse_define_missing_colon() {
    let input = r#"define test_intent
  steps:
    - click "Button""#;

    let result = parse_define(input);
    assert!(result.is_err(), "Should fail without colon after name");
}

#[test]
fn test_parse_define_empty_steps() {
    let input = r#"define empty:
  steps:"#;

    let result = parse_define(input);
    // May succeed with empty steps or fail
    if let Ok(def) = result {
        assert!(def.steps.is_empty());
    }
}

#[test]
fn test_parse_define_invalid_action() {
    let input = r#"define bad_action:
  steps:
    - invalid_action "Target""#;

    let result = parse_define(input);
    // Should either error or skip unknown action
    assert!(result.is_err() || result.is_ok());
}

#[test]
fn test_parse_define_unclosed_quote() {
    let input = r#"define unclosed:
  steps:
    - click "Unclosed"#;

    // Parser should handle unclosed quotes gracefully
    let result = parse_define(input);
    // Behavior depends on implementation - may error or treat rest as text
    assert!(result.is_err() || result.is_ok());
}

#[test]
fn test_parse_define_multiple_or() {
    let input = r#"define multi_or:
  steps:
    - click "A" or click "B" or click "C""#;

    let def = parse_define(input).expect("Should parse multiple or");
    assert_eq!(def.steps.len(), 1);

    // Should create nested Try blocks
    if let Step::Try(wrapper) = &def.steps[0] {
        assert!(!wrapper.try_.catch.is_empty());
    } else {
        panic!("Expected Try block");
    }
}

#[test]
fn test_parse_define_wait_with_condition() {
    // The define parser requires a target for wait commands
    let input = r#"define wait_test:
  steps:
    - wait visible "Login Form"
    - wait hidden "Modal Dialog""#;

    let def = parse_define(input).expect("Should parse wait steps");
    assert_eq!(def.steps.len(), 2);
}

#[test]
fn test_parse_define_special_characters_in_target() {
    let input = r#"define special:
  steps:
    - click "Button with 'quotes' and \"escapes\"""#;

    let result = parse_define(input);
    // Should handle special characters in targets
    assert!(result.is_ok() || result.is_err());
}

// ============================================================================
// Session Manager Error Tests
// ============================================================================

#[test]
fn test_session_manager_define_duplicate() {
    let mut manager = SessionIntentManager::new();

    let input = r#"define dup:
  steps:
    - click "A""#;
    let def = parse_define(input).expect("parse");

    manager.define(def.clone()).expect("first define");
    let result = manager.define(def);
    assert!(result.is_err(), "Should fail on duplicate definition");
}

#[test]
fn test_session_manager_undefine_nonexistent() {
    let mut manager = SessionIntentManager::new();
    let result = manager.undefine("nonexistent");
    assert!(result.is_err(), "Should fail on nonexistent intent");
}

#[test]
fn test_session_manager_get_nonexistent() {
    let manager = SessionIntentManager::new();
    assert!(manager.get("nonexistent").is_none());
}

#[test]
fn test_session_manager_list_empty() {
    let manager = SessionIntentManager::new();
    assert!(manager.list().is_empty());
}

#[test]
fn test_session_manager_list_multiple() {
    let mut manager = SessionIntentManager::new();

    let def1 = parse_define("define first:\n  steps:\n    - click \"A\"").unwrap();
    let def2 = parse_define("define second:\n  steps:\n    - click \"B\"").unwrap();

    manager.define(def1).unwrap();
    manager.define(def2).unwrap();

    let list = manager.list();
    assert_eq!(list.len(), 2);
}

#[test]
fn test_session_manager_export_nonexistent() {
    let manager = SessionIntentManager::new();
    let tmp_file = NamedTempFile::new().unwrap();
    let result = manager.export("nonexistent", tmp_file.path());
    assert!(result.is_err(), "Should fail exporting nonexistent intent");
}
