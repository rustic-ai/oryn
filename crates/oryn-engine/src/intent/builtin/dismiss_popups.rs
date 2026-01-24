use oryn_common::intent::definition::*;
use serde_json::json;

pub fn definition() -> IntentDefinition {
    IntentDefinition {
        name: "dismiss_popups".to_string(),
        description: None,
        version: "1.0.0".to_string(),
        tier: IntentTier::BuiltIn,
        triggers: IntentTriggers {
            patterns: vec![
                "modal_dialog".to_string(),
                "overlay".to_string(),
                "popup".to_string(),
            ],
            keywords: vec![
                "popup".to_string(),
                "modal".to_string(),
                "dismiss".to_string(),
            ],
            urls: vec![],
        },
        parameters: vec![ParameterDef {
            name: "all".to_string(),
            param_type: ParamType::Boolean,
            required: false,
            default: Some(json!(true)),
            description: "Dismiss all detected popups".to_string(),
        }],
        steps: vec![Step::Loop(LoopStepWrapper {
            loop_: LoopDef {
                over: "visible_patterns('modal_dialog', 'overlay', 'popup', 'cookie_banner')"
                    .to_string(),
                as_var: "popup".to_string(),
                steps: vec![Step::Try(TryStepWrapper {
                    try_: TryDef {
                        steps: vec![
                            Step::Action(ActionStep {
                                action: ActionType::Click,
                                on_error: None,
                                target: Some(TargetSpec {
                                    kind: TargetKind::Selector {
                                        selector: "$popup.close".to_string(),
                                    },
                                    fallback: Some(Box::new(TargetSpec {
                                        kind: TargetKind::Selector {
                                            selector: "button[aria-label*='lose']".to_string(),
                                        },
                                        fallback: Some(Box::new(TargetSpec {
                                            kind: TargetKind::Text {
                                                text: "✕".to_string(),
                                                match_type: MatchType::Contains,
                                            },
                                            fallback: Some(Box::new(TargetSpec {
                                                kind: TargetKind::Text {
                                                    text: "Close".to_string(),
                                                    match_type: MatchType::Contains,
                                                },
                                                fallback: Some(Box::new(TargetSpec {
                                                    kind: TargetKind::Text {
                                                         text: "Dismiss".to_string(),
                                                         match_type: MatchType::Contains,
                                                    },
                                                    fallback: Some(Box::new(TargetSpec {
                                                        kind: TargetKind::Text {
                                                            text: "No thanks".to_string(),
                                                            match_type: MatchType::Contains,
                                                        },
                                                        fallback: None,
                                                    })),
                                                })),
                                            })),
                                        })),
                                    })),
                                }),
                                options: Default::default(),
                            }),
                            Step::Action(ActionStep {
                                action: ActionType::Wait,
                                on_error: None,
                                target: None,
                                options: [(
                                    "condition".to_string(),
                                    json!({ "hidden": { "selector": "$popup" } }),
                                )]
                                .into(),
                            }),
                        ],
                        catch: vec![], // Continue if failed
                    },
                })],
                max: 5,
            },
        })],
        flow: None,
        success: None,
        failure: None,
        options: IntentOptions::default(),
    }
}
