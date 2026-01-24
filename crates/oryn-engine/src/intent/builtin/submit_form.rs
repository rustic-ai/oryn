use oryn_common::intent::definition::*;
use serde_json::json;

pub fn definition() -> IntentDefinition {
    IntentDefinition {
        name: "submit_form".to_string(),
        description: None,
        version: "1.0.0".to_string(),
        tier: IntentTier::BuiltIn,
        triggers: IntentTriggers {
            patterns: vec!["form".to_string()],
            keywords: vec!["submit".to_string()],
            urls: vec![],
        },
        parameters: vec![
            ParameterDef {
                name: "pattern".to_string(),
                param_type: ParamType::String,
                required: false,
                default: None,
                description: "Form pattern to target".to_string(),
            },
            ParameterDef {
                name: "wait".to_string(),
                param_type: ParamType::Number,
                required: false,
                default: Some(json!(10000)),
                description: "Time to wait (ms)".to_string(),
            },
        ],
        steps: vec![
            Step::Action(ActionStep {
                action: ActionType::Click,
                on_error: None,
                target: Some(TargetSpec {
                    kind: TargetKind::Pattern {
                        pattern: "$pattern.submit".to_string(),
                    },
                    fallback: Some(Box::new(TargetSpec {
                        kind: TargetKind::Role {
                            role: "submit".to_string(),
                        },
                        fallback: Some(Box::new(TargetSpec {
                            kind: TargetKind::Selector {
                                selector: "button[type='submit'], input[type='submit']".to_string(),
                            },
                            fallback: None,
                        })),
                    })),
                }),
                options: Default::default(),
            }),
            Step::Action(ActionStep {
                action: ActionType::Wait,
                on_error: None,
                target: None,
                options: [
                    ("condition".to_string(), json!({ "url_matches": ".*" })),
                    ("timeout".to_string(), json!("$wait")),
                ]
                .into(),
            }),
        ],
        flow: None,
        success: None,
        failure: None,
        options: IntentOptions::default(),
    }
}
