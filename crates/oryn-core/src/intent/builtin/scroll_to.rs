use crate::intent::definition::*;

pub fn definition() -> IntentDefinition {
    IntentDefinition {
        name: "scroll_to".to_string(),
        description: None,
        version: "1.0.0".to_string(),
        tier: IntentTier::BuiltIn,
        triggers: IntentTriggers {
            patterns: vec![],
            keywords: vec!["scroll".to_string()],
            urls: vec![],
        },
        parameters: vec![ParameterDef {
            name: "target".to_string(),
            param_type: ParamType::String,
            required: false,
            default: None,
            description: "Target to scroll to".to_string(),
        }],
        steps: vec![Step::Action(ActionStep {
            action: ActionType::Scroll,
            on_error: None,
            target: Some(TargetSpec {
                kind: TargetKind::Selector {
                    selector: "$target".to_string(),
                },
                fallback: None,
            }),
            options: Default::default(),
        })],
        success: None,
        failure: None,
        options: IntentOptions::default(),
    }
}
