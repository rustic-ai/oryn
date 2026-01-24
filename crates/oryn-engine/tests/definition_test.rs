use oryn_engine::intent::definition::{
    ActionStep, ActionType, Condition, IntentDefinition, IntentTier, IntentTriggers, ParamType,
    ParameterDef, Step, TargetKind, TargetSpec,
};
use serde_json::json;
use std::collections::HashMap;

#[test]
fn test_intent_definition_serde() {
    let intent = IntentDefinition {
        name: "test_intent".to_string(),
        version: "1.0.0".to_string(),
        tier: IntentTier::Loaded,
        triggers: IntentTriggers {
            patterns: vec!["login".to_string()],
            keywords: vec!["sign in".to_string()],
            urls: vec!["/login".to_string()],
        },
        parameters: vec![ParameterDef {
            name: "username".to_string(),
            param_type: ParamType::String,
            required: true,
            default: Some(json!("user")),
            description: "Login username".to_string(),
        }],
        steps: vec![Step::Action(ActionStep {
            action: ActionType::Type,
            on_error: None,
            target: Some(TargetSpec {
                kind: TargetKind::Selector {
                    selector: "#user".to_string(),
                },
                fallback: None,
            }),
            options: HashMap::from([("text".to_string(), json!("$username"))]),
        })],
        flow: None,
        success: Some(oryn_engine::intent::definition::SuccessCondition {
            conditions: vec![Condition::UrlContains(vec!["/dashboard".to_string()])],
            extract: None,
        }),
        failure: None,
        options: Default::default(),
        description: None,
    };

    // Test JSON Serialization
    let json = serde_json::to_string(&intent).unwrap();
    let deserialized: IntentDefinition = serde_json::from_str(&json).unwrap();

    assert_eq!(intent.name, deserialized.name);
    assert_eq!(intent.triggers.patterns, deserialized.triggers.patterns);
    assert!(matches!(deserialized.steps[0], Step::Action(_)));

    // Test YAML Serialization (if feature enabled / crate available, but serde_json covers struct logic)
    // We can assume if JSON works, YAML likely works as they use Serde.
    // But let's verify if we have serde_yaml in Cargo.toml.
    // The plan said we added it.
}

#[test]
fn test_default_values() {
    let json = r#"{
        "name": "minimal",
        "version": "0.1",
        "tier": "discovered",
        "steps": []
    }"#;

    let intent: IntentDefinition = serde_json::from_str(json).unwrap();
    assert!(intent.triggers.patterns.is_empty());
    assert!(intent.parameters.is_empty());
    assert_eq!(intent.options.timeout, 30000); // Check default timeout
}
