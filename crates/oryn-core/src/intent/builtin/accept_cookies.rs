use crate::intent::definition::*;
use serde_json::json;

pub fn definition() -> IntentDefinition {
    IntentDefinition {
        name: "accept_cookies".to_string(),
        description: None,
        version: "1.0.0".to_string(),
        tier: IntentTier::BuiltIn,
        triggers: IntentTriggers {
            patterns: vec!["cookie_banner".to_string()],
            keywords: vec!["cookie".to_string(), "consent".to_string()],
            urls: vec![],
        },
        parameters: vec![
            ParameterDef {
                name: "reject".to_string(),
                param_type: ParamType::Boolean,
                required: false,
                default: Some(json!(false)),
                description: "Click reject instead of accept".to_string(),
            },
        ],
        steps: vec![
            Step::Branch(BranchStepWrapper {
                branch: BranchDef {
                    condition: Condition::Expression("$reject".to_string()),
                    then_steps: vec![
                        Step::Action(ActionStep {
                            action: ActionType::Click, on_error: None,
                            target: Some(TargetSpec {
                                kind: TargetKind::Pattern { pattern: "cookie_banner.reject".to_string() },
                                fallback: Some(Box::new(TargetSpec {
                                     kind: TargetKind::Text { text: "Reject".to_string(), match_type: MatchType::Contains },
                                     fallback: None,
                                })),
                            }),
                            options: Default::default(),
                        }),
                    ],
                    else_steps: vec![
                        Step::Action(ActionStep {
                            action: ActionType::Click, on_error: None,
                            target: Some(TargetSpec {
                                kind: TargetKind::Pattern { pattern: "cookie_banner.accept".to_string() },
                                fallback: Some(Box::new(TargetSpec {
                                     kind: TargetKind::Text { text: "Accept".to_string(), match_type: MatchType::Contains },
                                     fallback: Some(Box::new(TargetSpec {
                                        kind: TargetKind::Text { text: "Allow".to_string(), match_type: MatchType::Contains },
                                        fallback: Some(Box::new(TargetSpec {
                                            // Fallback to simpler lookups if needed
                                            kind: TargetKind::Selector { selector: "button:contains('Accept'), button:contains('Allow')".to_string() }, // Pseudo selector not real css
                                            fallback: None
                                        }))
                                     })),
                                })),
                            }),
                            options: Default::default(),
                        }),
                    ],
                }
            }),
            Step::Action(ActionStep {
                action: ActionType::Wait, on_error: None,
                target: None,
                options: [
                    ("condition".to_string(), json!({ "hidden": { "pattern": "cookie_banner" } })),
                    ("timeout".to_string(), json!(2000)),
                ].into(),
            }),
        ],
        success: Some(SuccessCondition {
            conditions: vec![
                 Condition::Hidden(TargetSpec { kind: TargetKind::Pattern { pattern: "cookie_banner".to_string() }, fallback: None }),
            ],
            extract: None,
        }),
        failure: None,
        options: IntentOptions::default(),
    }
}
