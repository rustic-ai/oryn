use crate::intent::definition::*;
use serde_json::json;

pub fn definition() -> IntentDefinition {
    IntentDefinition {
        name: "logout".to_string(),
        version: "1.0.0".to_string(),
        tier: IntentTier::BuiltIn,
        triggers: IntentTriggers {
            patterns: vec!["user_menu".to_string()],
            keywords: vec![
                "logout".to_string(),
                "sign out".to_string(),
                "log out".to_string(),
            ],
            urls: vec![],
        },
        parameters: vec![ParameterDef {
            name: "wait".to_string(),
            param_type: ParamType::Number,
            required: false,
            default: Some(json!(5000)),
            description: "Time to wait (ms)".to_string(),
        }],
        steps: vec![
            Step::Try(TryStepWrapper {
                try_: TryDef {
                    steps: vec![
                        // Try clicking logout directly
                        Step::Action(ActionStep {
                            action: ActionType::Click,
                            target: Some(TargetSpec {
                                kind: TargetKind::Pattern {
                                    pattern: "logout_button".to_string(),
                                },
                                fallback: Some(Box::new(TargetSpec {
                                    kind: TargetKind::Text {
                                        text: "Sign out".to_string(),
                                        match_type: MatchType::Contains,
                                    },
                                    fallback: Some(Box::new(TargetSpec {
                                        kind: TargetKind::Text {
                                            text: "Log out".to_string(),
                                            match_type: MatchType::Contains,
                                        },
                                        fallback: None,
                                    })),
                                })),
                            }),
                            options: Default::default(),
                        }),
                    ],
                    catch: vec![
                        // Try opening user menu first
                        Step::Action(ActionStep {
                            action: ActionType::Click,
                            target: Some(TargetSpec {
                                kind: TargetKind::Pattern {
                                    pattern: "user_menu".to_string(),
                                },
                                fallback: Some(Box::new(TargetSpec {
                                    kind: TargetKind::Role {
                                        role: "avatar".to_string(),
                                    },
                                    fallback: None,
                                })),
                            }),
                            options: Default::default(),
                        }),
                        Step::Action(ActionStep {
                            action: ActionType::Click,
                            target: Some(TargetSpec {
                                kind: TargetKind::Text {
                                    text: "Sign out".to_string(),
                                    match_type: MatchType::Contains,
                                },
                                fallback: Some(Box::new(TargetSpec {
                                    kind: TargetKind::Text {
                                        text: "Log out".to_string(),
                                        match_type: MatchType::Contains,
                                    },
                                    fallback: None,
                                })),
                            }),
                            options: Default::default(),
                        }),
                    ],
                },
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
