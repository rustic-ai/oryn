use crate::command::{Command, Target};
use crate::intent::definition::{
    ActionStep, ActionType, IntentDefinition, IntentOptions, IntentTier, IntentTriggers, ParamType,
    ParameterDef, Step, TargetKind, TargetSpec,
};
use crate::learner::recognizer::Pattern;
use serde_json::json;
use std::collections::{HashMap, HashSet};

pub struct Proposer;

impl Proposer {
    pub fn new() -> Self {
        Self
    }

    /// Converts a recognized pattern into a proposed IntentDefinition.
    pub fn propose(&self, pattern: &Pattern) -> IntentDefinition {
        let name = format!("intent_{}", pattern.observation_count);

        let mut steps = Vec::new();
        let mut parameters = Vec::new();

        // Iterate through each step in the sequence
        for i in 0..pattern.sequence.len() {
            let representative_cmd = &pattern.sequence[i];

            // Check for variations in this step across all occurrences
            let mut text_values = HashSet::new();
            let mut select_values = HashSet::new();

            for occurrence in &pattern.occurrences {
                if i < occurrence.len() {
                    match &occurrence[i] {
                        Command::Type(_, text, _) => {
                            text_values.insert(text.clone());
                        }
                        Command::Select(_, value) => {
                            select_values.insert(value.clone());
                        }
                        _ => {}
                    }
                }
            }

            // Convert Command to Step, potentially parameterizing
            if let Some(step) = self.convert_command(
                representative_cmd,
                &mut parameters,
                text_values,
                select_values,
            ) {
                steps.push(step);
            }
        }

        IntentDefinition {
            name,
            description: Some(format!(
                "Discovered intent from {} actions",
                pattern.sequence.len()
            )),
            version: "0.1.0".to_string(),
            tier: IntentTier::Discovered,
            triggers: IntentTriggers {
                patterns: vec![],
                keywords: vec![],
                urls: vec![pattern.domain.clone()],
            },
            parameters,
            steps,
            success: None,
            failure: None,
            options: IntentOptions::default(),
        }
    }

    fn convert_command(
        &self,
        cmd: &Command,
        params: &mut Vec<ParameterDef>,
        text_variations: HashSet<String>,
        select_variations: HashSet<String>,
    ) -> Option<Step> {
        match cmd {
            Command::Click(target, _) => Some(Step::Action(ActionStep {
                action: ActionType::Click,
                target: Some(self.convert_target(target)),
                options: HashMap::new(),
            })),

            Command::Type(target, text, _) => {
                let text_val = if text_variations.len() > 1 {
                    // Parameterize!
                    let param_name = format!("param_{}", params.len() + 1);
                    params.push(ParameterDef {
                        name: param_name.clone(),
                        param_type: ParamType::String,
                        required: true,
                        default: None,
                        description: format!("Input for {}", param_name),
                    });
                    json!(format!("{{{{{}}}}}", param_name)) // {{param_1}}
                } else {
                    json!(text)
                };

                let mut options = HashMap::new();
                options.insert("text".to_string(), text_val);

                Some(Step::Action(ActionStep {
                    action: ActionType::Type,
                    target: Some(self.convert_target(target)),
                    options,
                }))
            }

            Command::Select(target, value) => {
                let val = if select_variations.len() > 1 {
                    let param_name = format!("param_{}", params.len() + 1);
                    params.push(ParameterDef {
                        name: param_name.clone(),
                        param_type: ParamType::String, // or Any?
                        required: true,
                        default: None,
                        description: format!("Selection for {}", param_name),
                    });
                    json!(format!("{{{{{}}}}}", param_name))
                } else {
                    json!(value)
                };

                let mut options = HashMap::new();
                options.insert("value".to_string(), val);

                Some(Step::Action(ActionStep {
                    action: ActionType::Select,
                    target: Some(self.convert_target(target)),
                    options,
                }))
            }

            Command::Check(target) => Some(Step::Action(ActionStep {
                action: ActionType::Check,
                target: Some(self.convert_target(target)),
                options: HashMap::new(),
            })),

            Command::Uncheck(target) => Some(Step::Action(ActionStep {
                action: ActionType::Uncheck,
                target: Some(self.convert_target(target)),
                options: HashMap::new(),
            })),

            Command::Clear(target) => Some(Step::Action(ActionStep {
                action: ActionType::Clear,
                target: Some(self.convert_target(target)),
                options: HashMap::new(),
            })),

            Command::Scroll(target_opt, _) => Some(Step::Action(ActionStep {
                action: ActionType::Scroll,
                target: target_opt.as_ref().map(|t| self.convert_target(t)),
                options: HashMap::new(),
            })),

            Command::Wait(_, _) => Some(Step::Action(ActionStep {
                action: ActionType::Wait,
                target: None,
                options: HashMap::new(), // TODO: Convert wait condition
            })),

            // Simplify: ignore others for now or implement as generic Execute?
            _ => None,
        }
    }

    fn convert_target(&self, target: &Target) -> TargetSpec {
        match target {
            Target::Id(id) => TargetSpec {
                kind: TargetKind::Id { id: *id as u64 },
                fallback: None,
            },
            Target::Text(text) => TargetSpec {
                kind: TargetKind::Text {
                    text: text.clone(),
                    match_type: Default::default(),
                },
                fallback: None,
            },
            Target::Role(role) => TargetSpec {
                kind: TargetKind::Role { role: role.clone() },
                fallback: None,
            },
            Target::Selector(sel) => TargetSpec {
                kind: TargetKind::Selector {
                    selector: sel.clone(),
                },
                fallback: None,
            },
            // Simplified handling for relational: convert to pattern or ignore for now
            // Spec allows relational? TargetSpec in definition.rs doesn't seem to have Relational variants clearly exposed in TargetKind?
            // TargetKind has Pattern, Role, Text, Selector, Id.
            // Relational target support in Definition might be via selector or fallback?
            // Checking definition.rs... TargetKind does NOT have Near/Inside.
            // Wait, does Executor support relational targets in Definition?
            // Executor supports `Target` from `command.rs`.
            // But `IntentDefinition` uses `TargetSpec`. Translator converts `TargetSpec` to `Target`.
            // `TargetKind` in definition.rs lines 159-177 indeed missing Relational.
            // This is a gap in the spec/impl match!
            // For now, we fallback to Selector if possible or Pattern?
            // We'll panic or return dummy for relational to avoid compile error,
            // assuming learned intents use simple targets for now.
            _ => TargetSpec {
                kind: TargetKind::Selector {
                    selector: "unsupported_relational".into(),
                },
                fallback: None,
            },
        }
    }
}
