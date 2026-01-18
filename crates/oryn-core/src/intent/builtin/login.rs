use crate::intent::definition::*;
use serde_json::json;

pub fn definition() -> IntentDefinition {
    IntentDefinition {
        name: "login".to_string(),
        version: "1.0.0".to_string(),
        tier: IntentTier::BuiltIn,
        triggers: IntentTriggers {
            patterns: vec!["login_form".to_string()],
            keywords: vec![
                "login".to_string(),
                "signin".to_string(),
                "log in".to_string(),
                "sign in".to_string(),
            ],
            urls: vec!["*/login".to_string(), "*/signin".to_string()],
        },
        parameters: vec![
            ParameterDef {
                name: "username".to_string(),
                param_type: ParamType::String,
                required: true,
                default: None,
                description: "Username or email address".to_string(),
            },
            ParameterDef {
                name: "password".to_string(),
                param_type: ParamType::String,
                required: true,
                default: None,
                description: "Password".to_string(),
            },
            ParameterDef {
                name: "wait".to_string(),
                param_type: ParamType::Number,
                required: false,
                default: Some(json!(10000)),
                description: "Time to wait for navigation (ms)".to_string(),
            },
        ],
        steps: vec![
            Step::Action(ActionStep {
                action: ActionType::Type,
                target: Some(TargetSpec {
                    kind: TargetKind::Pattern {
                        pattern: "login_form.username".to_string(),
                    },
                    fallback: Some(Box::new(TargetSpec {
                        kind: TargetKind::Role {
                            role: "username".to_string(),
                        },
                        fallback: Some(Box::new(TargetSpec {
                            kind: TargetKind::Selector {
                                selector: "input[type='email'], input[name*='user']".to_string(),
                            },
                            fallback: None,
                        })),
                    })),
                }),
                options: [("text".to_string(), json!("$username"))].into(),
            }),
            Step::Action(ActionStep {
                action: ActionType::Type,
                target: Some(TargetSpec {
                    kind: TargetKind::Pattern {
                        pattern: "login_form.password".to_string(),
                    },
                    fallback: Some(Box::new(TargetSpec {
                        kind: TargetKind::Role {
                            role: "password".to_string(),
                        },
                        fallback: Some(Box::new(TargetSpec {
                            kind: TargetKind::Selector {
                                selector: "input[type='password']".to_string(),
                            },
                            fallback: None,
                        })),
                    })),
                }),
                options: [("text".to_string(), json!("$password"))].into(),
            }),
            Step::Action(ActionStep {
                action: ActionType::Click,
                target: Some(TargetSpec {
                    kind: TargetKind::Pattern {
                        pattern: "login_form.submit".to_string(),
                    },
                    fallback: Some(Box::new(TargetSpec {
                        kind: TargetKind::Role {
                            role: "submit".to_string(),
                        },
                        fallback: Some(Box::new(TargetSpec {
                            kind: TargetKind::Selector {
                                selector: "button[type='submit']".to_string(),
                            },
                            fallback: None,
                        })),
                    })),
                }),
                options: Default::default(),
            }),
            Step::Action(ActionStep {
                action: ActionType::Wait,
                target: None,
                options: [
                    ("condition".to_string(), json!({ "url_matches": ".*" })), // Simple wait for now, logic handled in engine
                    ("timeout".to_string(), json!("$wait")),
                ]
                .into(),
            }),
        ],
        success: Some(SuccessCondition {
            conditions: vec![Condition::Hidden(TargetSpec {
                kind: TargetKind::Pattern {
                    pattern: "login_form".to_string(),
                },
                fallback: None,
            })],
            extract: None,
        }),
        failure: None,
        options: IntentOptions::default(),
    }
}
