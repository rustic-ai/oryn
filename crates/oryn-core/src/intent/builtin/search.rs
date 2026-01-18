use crate::intent::definition::*;
use serde_json::json;

pub fn definition() -> IntentDefinition {
    IntentDefinition {
        name: "search".to_string(),
        description: None,
        version: "1.0.0".to_string(),
        tier: IntentTier::BuiltIn,
        triggers: IntentTriggers {
            patterns: vec!["search_form".to_string()],
            keywords: vec!["search".to_string(), "find".to_string()],
            urls: vec![],
        },
        parameters: vec![
            ParameterDef {
                name: "query".to_string(),
                param_type: ParamType::String,
                required: true,
                default: None,
                description: "Search terms".to_string(),
            },
            ParameterDef {
                name: "wait".to_string(),
                param_type: ParamType::Number,
                required: false,
                default: Some(json!(5000)),
                description: "Time to wait for results (ms)".to_string(),
            },
        ],
        steps: vec![
            Step::Action(ActionStep {
                action: ActionType::Clear,
                target: Some(TargetSpec {
                    kind: TargetKind::Pattern {
                        pattern: "search_form.input".to_string(),
                    },
                    fallback: Some(Box::new(TargetSpec {
                        kind: TargetKind::Role {
                            role: "search".to_string(),
                        },
                        fallback: Some(Box::new(TargetSpec {
                            kind: TargetKind::Selector {
                                selector:
                                    "input[type='search'], input[name*='q'], input[name*='search']"
                                        .to_string(),
                            },
                            fallback: None,
                        })),
                    })),
                }),
                options: Default::default(),
            }),
            Step::Action(ActionStep {
                action: ActionType::Type,
                target: Some(TargetSpec {
                    kind: TargetKind::Pattern {
                        pattern: "search_form.input".to_string(),
                    },
                    fallback: Some(Box::new(TargetSpec {
                        kind: TargetKind::Role {
                            role: "search".to_string(),
                        },
                        fallback: Some(Box::new(TargetSpec {
                            kind: TargetKind::Selector {
                                selector:
                                    "input[type='search'], input[name*='q'], input[name*='search']"
                                        .to_string(),
                            },
                            fallback: None,
                        })),
                    })),
                }),
                options: [
                    ("text".to_string(), json!("$query")),
                    ("enter".to_string(), json!(true)),
                ]
                .into(),
            }),
            Step::Action(ActionStep {
                action: ActionType::Wait,
                target: None,
                options: [
                    ("condition".to_string(), json!({ "url_matches": ".*" })),
                    ("timeout".to_string(), json!("$wait")),
                ]
                .into(),
            }),
        ],
        success: None,
        failure: None,
        options: IntentOptions::default(),
    }
}
