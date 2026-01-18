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
