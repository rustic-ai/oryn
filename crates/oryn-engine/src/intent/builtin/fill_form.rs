use oryn_common::intent::definition::*;
use serde_json::json;

pub fn definition() -> IntentDefinition {
    IntentDefinition {
        name: "fill_form".to_string(),
        description: None,
        version: "1.0.0".to_string(),
        tier: IntentTier::BuiltIn,
        triggers: IntentTriggers {
            patterns: vec!["form".to_string()],
            keywords: vec!["fill".to_string(), "form".to_string()],
            urls: vec![],
        },
        parameters: vec![
            ParameterDef {
                name: "data".to_string(),
                param_type: ParamType::Object,
                required: true,
                default: None,
                description: "Data to fill".to_string(),
            },
            ParameterDef {
                name: "pattern".to_string(),
                param_type: ParamType::String,
                required: false,
                default: None,
                description: "Form pattern to target".to_string(),
            },
        ],
        steps: vec![Step::Action(ActionStep {
            action: ActionType::FillForm,
            on_error: None,
            target: Some(TargetSpec {
                kind: TargetKind::Pattern {
                    pattern: "$pattern".to_string(),
                },
                fallback: Some(Box::new(TargetSpec {
                    kind: TargetKind::Selector {
                        selector: "form".to_string(),
                    },
                    fallback: None,
                })),
            }),
            options: [("data".to_string(), json!("$data"))].into(),
        })],
        flow: None,
        success: None,
        failure: None,
        options: IntentOptions::default(),
    }
}
